use std::env;

use aws_sdk_s3::primitives::ByteStream;
use axum::{
    body::{ Body, Bytes },
    extract::{ DefaultBodyLimit, Path, Request, State },
    http::{ HeaderMap, HeaderValue },
    middleware::{ from_fn_with_state, Next },
    response::{ IntoResponse, Response },
    routing::post,
    Router,
};
use axum_extra::extract::CookieJar;
use axum_typed_multipart::{ FieldData, TryFromMultipart, TypedMultipart };
use reqwest::{ header::CONTENT_TYPE, StatusCode };
use uuid::Uuid;

use crate::{
    enums::ImageType,
    state::models::{ AppState, PermissionCheckResponse },
    utils::{ auth_utils::check_auth, db_utils::get_client, image_utils::encode_lossy_webp },
    MAX_FILE_SIZE,
};

#[derive(TryFromMultipart)]
struct UpdatePayload {
    title: Option<String>,
    owner_id: Option<Uuid>,
    #[form_data(limit = "10MiB")]
    file: Option<FieldData<Bytes>>,
}

async fn update_asset(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    TypedMultipart(UpdatePayload { title, owner_id, file }): TypedMultipart<UpdatePayload>
) -> impl IntoResponse {
    let client = get_client(&state.pool).await;

    if client.is_err() {
        return client.err().unwrap();
    }
    let client = client.unwrap();
    if title.is_some() || owner_id.is_some() {
        if title.is_some() && owner_id.is_some() {
            let res = client.query(
                "UPDATE images SET title = $1, owner_id = $2 WHERE id = $3",
                &[&title.unwrap(), &owner_id.unwrap(), &id]
            ).await;

            if res.is_err() {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "There was an error with your request".to_string(),
                );
            }
        } else if title.is_some() && owner_id.is_none() {
            let res = client.query(
                "UPDATE images SET title = $1 WHERE id = $2",
                &[&title.unwrap(), &id]
            ).await;

            if res.is_err() {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "There was an error with your request".to_string(),
                );
            }
        } else if title.is_none() && owner_id.is_some() {
            let res = client.query(
                "UPDATE images SET owner_id = $1 WHERE id = $2",
                &[&owner_id.unwrap(), &id]
            ).await;

            if res.is_err() {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "There was an error with your request".to_string(),
                );
            }
        }
    }

    if file.is_some() {
        let current_image = client.query_one(
            "SELECT project_id, type FROM images WHERE id = $1;",
            &[&id]
        ).await;

        if current_image.is_err() {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "There was an error with your request".to_string(),
            );
        }

        let current_image = current_image.unwrap();

        let project_id: Uuid = current_image.get("project_id");
        let image_type: ImageType = current_image.get("type");

        let file = file.unwrap();

        let img_data = image::load_from_memory(&file.contents);

        if img_data.is_err() {
            println!("{}", img_data.err().unwrap());
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "There was an error with your request".to_string(),
            );
        }

        let lossy = encode_lossy_webp(img_data.unwrap());

        let upload = state.client
            .put_object()
            .bucket(&state.bucket)
            .key(format!("assets/{}/{}/{}.webp", &project_id, &image_type, &id))
            .body(ByteStream::from(lossy))
            .acl(aws_sdk_s3::types::ObjectCannedAcl::Private)
            .content_type("image/webp")
            .cache_control("max-age=600")
            .send().await;

        if upload.is_err() {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "There was an error with your request".to_string(),
            );
        }
    }

    return (StatusCode::OK, "Stagod".to_string());
}

async fn permission_middleware(
    cookie_jar: CookieJar,
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
    request: Request,
    next: Next
) -> Response {
    let url = request.uri().to_string();
    let action = match url {
        u if u.contains("/update/") => "update",
        u if u.contains("/delete/") => "delete",
        u if u.contains("upload") => "upload",
        _ => "NONE",
    };

    if action == "NONE" {
        let res = Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(Body::from("There was an error with your request."))
            .unwrap();

        return res;
    }

    let claims = check_auth(cookie_jar, &state.reqwest_client, state.auth_service_url).await;

    if claims.is_err() {
        let res = Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(Body::from("There was an error with your request."))
            .unwrap();

        return res;
    }

    let claims = claims.unwrap().claims;

    if claims.is_none() {
        let res = Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(Body::from("There was an error with your request."))
            .unwrap();

        return res;
    }

    let claims = claims.unwrap();

    let mut headers = HeaderMap::new();

    headers.append(CONTENT_TYPE, HeaderValue::from_str("application/json").unwrap());
    headers.append("user-id", HeaderValue::from_str(claims.user_id.to_string().as_str()).unwrap());
    headers.append(
        "project-id",
        HeaderValue::from_str(claims.project_id.to_string().as_str()).unwrap()
    );

    let auth_service_url = env::var("AUTH_SERVICE_URL").expect("NO AUTH SERVICE");
    let res = state.reqwest_client
        .get(format!("{}/auth/permission/{}_images", auth_service_url, &action))
        .headers(headers)
        .send().await;

    if res.is_err() {
        let res = Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(Body::from("There was an error with your request."))
            .unwrap();

        return res;
    }

    let permissions = res.unwrap().json::<PermissionCheckResponse>().await;

    if permissions.is_err() {
        let res = Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(Body::from("There was an error with your request."))
            .unwrap();

        return res;
    }

    let permissions = permissions.unwrap();
    let client = get_client(&state.pool).await;

    if client.is_err() {
        let res = Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(Body::from("There was an error with your request."))
            .unwrap();

        return res;
    }
    let client = client.unwrap();

    let has_permission = match permissions.is_project_owner {
        true => true,
        false => {
            let permission_check = client.query_opt(
                "SELECT TRUE AS has_permission
                 FROM images
                 LEFT JOIN entity_permissions ON entity_permissions.related_id = images.id
                 WHERE images.id = $1
                    AND
                        (images.owner_id = $2
                    OR
                        entity_permissions.role_id = $3
                    OR
                        (entity_permissions.user_id = $2 AND entity_permissions.permission_id = $4 AND entity_permissions.related_id = images.id)
                    );",
                &[&id, &claims.user_id, &permissions.role_id, &permissions.permission_id]
            ).await;

            if permission_check.is_err() {
                false;
            }

            let permission_check = permission_check.unwrap();

            permission_check.is_some()
        }
    };

    if !has_permission {
        let res = Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(Body::from("There was an error with your request."))
            .unwrap();

        return res;
    }
    return next.run(request).await;
}

pub fn crud_routes(state: AppState) -> Router<AppState> {
    Router::new().nest(
        "/assets",
        Router::new()
            .route("/update/:id", post(update_asset))
            .layer(from_fn_with_state(state, permission_middleware))
            .layer(DefaultBodyLimit::max(MAX_FILE_SIZE))
    )
}
