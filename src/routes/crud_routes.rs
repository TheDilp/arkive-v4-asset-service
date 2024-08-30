use std::{ env, str::FromStr };

use aws_sdk_s3::{ primitives::ByteStream, types::ObjectIdentifier };
use axum::{
    body::{ Body, Bytes },
    extract::{ DefaultBodyLimit, Request, State },
    http::{ HeaderMap, HeaderValue },
    middleware::{ from_fn_with_state, Next },
    response::{ IntoResponse, Response },
    routing::{ delete, post },
    Json,
    Router,
};
use axum_extra::extract::CookieJar;
use axum_typed_multipart::{ FieldData, TryFromMultipart, TypedMultipart };
use deadpool_postgres::GenericClient;
use reqwest::{ header::CONTENT_TYPE, Method, StatusCode };
use serde::Deserialize;
use base64::prelude::*;

use serde_json::json;
use uuid::Uuid;

use crate::{
    enums::{ AppResponse, ImageType },
    state::models::{ AppState, PermissionCheckResponse },
    utils::{
        auth_utils::check_auth,
        db_utils::get_client,
        extractors::ExtractPath,
        image_utils::encode_lossy_webp,
        s3_utils::recursive_delete,
    },
    MAX_FILE_SIZE,
};

#[derive(Deserialize)]
struct PermissionUpdateType {
    related_id: Uuid,
    user_id: Option<Uuid>,
    permission_id: Option<Uuid>,
    role_id: Option<Uuid>,
}

#[derive(TryFromMultipart)]
struct UpdatePayload {
    title: Option<String>,
    owner_id: Option<Uuid>,
    #[form_data(limit = "10MiB")]
    file: Option<FieldData<Bytes>>,
    permissions: Option<String>,
}

#[derive(Deserialize)]
struct ImageDownload {
    id: Uuid,
}

#[derive(Deserialize)]
struct DownloadPayload {
    data: Vec<ImageDownload>,
}

#[derive(Deserialize)]
struct ImageDelete {
    ids: Vec<Uuid>,
    project_id: Uuid,
}

#[derive(Deserialize)]
struct BulkDeletePayload {
    data: ImageDelete,
}

async fn update_asset(
    State(state): State<AppState>,
    ExtractPath(id): ExtractPath<Uuid>,
    TypedMultipart(
        UpdatePayload { title, owner_id, permissions, file },
    ): TypedMultipart<UpdatePayload>
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
                return AppResponse::Error(res.err().unwrap().to_string());
            }
        } else if title.is_some() && owner_id.is_none() {
            let res = client.query(
                "UPDATE images SET title = $1 WHERE id = $2",
                &[&title.unwrap(), &id]
            ).await;

            if res.is_err() {
                return AppResponse::Error(res.err().unwrap().to_string());
            }
        } else if title.is_none() && owner_id.is_some() {
            let res = client.query(
                "UPDATE images SET owner_id = $1 WHERE id = $2",
                &[&owner_id.unwrap(), &id]
            ).await;

            if res.is_err() {
                return AppResponse::Error(res.err().unwrap().to_string());
            }
        }
    }

    if file.is_some() {
        let current_image = client.query_one(
            "SELECT project_id, type FROM images WHERE id = $1;",
            &[&id]
        ).await;

        if current_image.is_err() {
            return AppResponse::Error(current_image.err().unwrap().to_string());
        }

        let current_image = current_image.unwrap();

        let project_id: Uuid = current_image.get("project_id");
        let image_type: ImageType = current_image.get("type");

        let file = file.unwrap();

        let img_data = image::load_from_memory(&file.contents);

        if img_data.is_err() {
            return AppResponse::Error(img_data.err().unwrap().to_string());
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
            return AppResponse::Error(upload.err().unwrap().to_string());
        }
    }

    if permissions.is_some() {
        let permissions = permissions.unwrap();

        let permissions: Vec<PermissionUpdateType> = serde_json::from_str(&permissions).unwrap();

        for perm in permissions {
            if perm.role_id.is_some() && perm.permission_id.is_none() && perm.user_id.is_none() {
                let client = get_client(&state.pool).await;
                if client.is_err() {
                    tracing::error!("Error constructing client - PERMISSIONS TRANSACTION.");
                    return AppResponse::Success(
                        "Image".to_owned(),
                        crate::enums::SuccessActions::Update
                    );
                }
                let mut client = client.unwrap();

                let transaction = client.transaction().await.unwrap();

                let del_res = transaction.execute(
                    "DELETE FROM entity_permissions WHERE id = $1",
                    &[&perm.related_id]
                ).await;

                if del_res.is_ok() {
                    let insert_res = transaction.execute(
                        "INSERT INTO entity_permissions (related_id, role_id) VALUES ($1, $2) ON CONFLICT (related_id, role_id) DO UPDATE SET role_id = $2;",
                        &[&perm.related_id, &perm.role_id.unwrap()]
                    ).await;

                    if insert_res.is_ok() {
                        let transaction_result = transaction.commit().await;

                        if transaction_result.is_err() {
                            tracing::error!(
                                "Transaction error - {}",
                                transaction_result.err().unwrap().to_string()
                            );
                        }
                    } else {
                        tracing::error!(
                            "Transaction error - {}",
                            insert_res.err().unwrap().to_string()
                        );
                    }
                } else {
                    tracing::error!("Transaction error - {}", del_res.err().unwrap().to_string());
                }
            } else if perm.permission_id.is_some() && perm.user_id.is_some() {
                let client = get_client(&state.pool).await;
                if client.is_err() {
                    tracing::error!("Error constructing client - PERMISSIONS TRANSACTION.");
                    return AppResponse::Success(
                        "Image".to_owned(),
                        crate::enums::SuccessActions::Update
                    );
                }
                let mut client = client.unwrap();

                let transaction = client.transaction().await.unwrap();

                let del_res = transaction.execute(
                    "DELETE FROM entity_permissions WHERE id = $1",
                    &[&perm.related_id]
                ).await;

                if del_res.is_ok() {
                    let insert_res = transaction.execute(
                        "INSERT INTO entity_permissions (related_id, permission_id, user_id) VALUES ($1, $2, $3) ON CONFLICT (user_id, related_id, permission_id) DO NOTHING;",
                        &[&perm.related_id, &perm.permission_id.unwrap(), &perm.user_id.unwrap()]
                    ).await;

                    if insert_res.is_ok() {
                        let transaction_result = transaction.commit().await;

                        if transaction_result.is_err() {
                            tracing::error!(
                                "Transaction error - {}",
                                transaction_result.err().unwrap().to_string()
                            );
                        }
                    } else {
                        tracing::error!(
                            "Transaction error - {}",
                            insert_res.err().unwrap().to_string()
                        );
                    }
                } else {
                    tracing::error!("Transaction error - {}", del_res.err().unwrap().to_string());
                }
            }
        }
    }

    return AppResponse::Success("Image".to_owned(), crate::enums::SuccessActions::Update);
}

async fn delete_asset(
    State(state): State<AppState>,
    ExtractPath((project_id, image_type, id)): ExtractPath<(Uuid, ImageType, Uuid)>
) -> impl IntoResponse {
    let client = get_client(&state.pool).await;

    if client.is_err() {
        return client.err().unwrap();
    }
    let client = client.unwrap();

    let del_res = &state.client
        .delete_object()
        .bucket(&state.bucket)
        .key(format!("assets/{}/{}/{}.webp", &project_id, &image_type, &id))
        .send().await;

    if del_res.is_err() {
        tracing::error!("{}", del_res.as_ref().err().unwrap());
    }

    let res = client.query("DELETE FROM images WHERE id = $1;", &[&id]).await;

    if res.is_err() {
        return AppResponse::Error(res.err().unwrap().to_string());
    }

    return AppResponse::Success("Image".to_owned(), crate::enums::SuccessActions::Delete);
}

async fn bulk_delete_assets(
    State(state): State<AppState>,
    ExtractPath(image_type): ExtractPath<ImageType>,
    Json(payload): Json<BulkDeletePayload>
) -> impl IntoResponse {
    let client = get_client(&state.pool).await;

    if client.is_err() {
        return client.err().unwrap();
    }
    let client = client.unwrap();

    let res = client.query(
        "DELETE FROM images WHERE id = ANY($1) RETURNING id;",
        &[&payload.data.ids]
    ).await;

    if res.is_err() {
        return AppResponse::Error(res.err().unwrap().to_string());
    }

    let deleted_ids: Vec<Uuid> = res
        .unwrap()
        .iter()
        .map(|row| row.get("id"))
        .collect();

    let mut delete_objects: Vec<ObjectIdentifier> = vec![];
    for id in deleted_ids {
        let obj_id = ObjectIdentifier::builder()
            .set_key(
                Some(format!("assets/{}/{}/{}.webp", &payload.data.project_id, &image_type, &id))
            )
            .build();

        if obj_id.is_err() {
            continue;
        }

        let obj_id = obj_id.unwrap();

        delete_objects.push(obj_id);
    }

    let delete_cmd = aws_sdk_s3::types::Delete::builder().set_objects(Some(delete_objects)).build();

    if delete_cmd.is_err() {
        return AppResponse::Error(delete_cmd.err().unwrap().to_string());
    }
    let delete_cmd = delete_cmd.unwrap();
    let delete_res = &state.client
        .delete_objects()
        .bucket(&state.bucket)
        .delete(delete_cmd)
        .send().await;

    if delete_res.is_err() {
        AppResponse::Error(delete_res.as_ref().err().unwrap().to_string());
    }

    return AppResponse::Success("Images".to_owned(), crate::enums::SuccessActions::Delete);
}

async fn download_assets(
    State(state): State<AppState>,
    ExtractPath((project_id, image_type)): ExtractPath<(Uuid, ImageType)>,
    Json(payload): Json<DownloadPayload>
) -> impl IntoResponse {
    let mut data_strings: Vec<String> = Vec::new();
    for image in payload.data {
        let data = state.client
            .get_object()
            .bucket(&state.bucket)
            .key(format!("assets/{}/{}/{}.webp", &project_id, &image_type, &image.id))
            .send().await;

        if data.is_err() {
            tracing::error!("ERROR GETTING IMAGE DATA - {}", data.err().unwrap());
            continue;
        }

        let data = data.unwrap().body.collect().await;

        if data.is_err() {
            tracing::error!("ERROR GETTING IMAGE DATA - {}", data.err().unwrap());
            continue;
        }

        let data = data.unwrap().into_bytes();

        let base_64 = BASE64_STANDARD.encode(data);

        data_strings.push(base_64);
    }
    return AppResponse::SuccessData(
        "Assets".to_owned(),
        crate::enums::SuccessActions::Download,
        json!(data_strings)
    );
}

async fn permission_middleware(
    cookie_jar: CookieJar,
    State(state): State<AppState>,
    request: Request,
    next: Next
) -> Response {
    let url = request.uri().to_string();
    let id = Uuid::from_str(url.clone().split("/").last().unwrap());

    if id.is_err() {
        return AppResponse::Error(id.err().unwrap().to_string()).into_response();
    }
    let id = id.unwrap();

    let action = match url {
        u if u.contains("/update/") => "update",
        u if u.contains("/delete/") || request.method() == &Method::DELETE => "delete",
        u if u.contains("upload") => "upload",
        u if u.contains("download") => "read",
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
        return AppResponse::Auth.into_response();
    }

    return next.run(request).await;
}

async fn delete_folder(
    State(state): State<AppState>,
    ExtractPath(project_id): ExtractPath<Uuid>
) -> impl IntoResponse {
    let location = format!("assets/{}", project_id);

    let res = recursive_delete(&state.client, &state.bucket, &location).await;

    if res.is_err() {
        return res.err().unwrap();
    }

    let client = get_client(&state.pool).await;

    if client.is_err() {
        return client.err().unwrap();
    }
    let client = client.unwrap();

    let img_delete_res = client.query(
        "DELETE FROM images WHERE project_id = $1;",
        &[&project_id]
    ).await;

    if img_delete_res.is_err() {
        return AppResponse::Error(img_delete_res.err().unwrap().to_string());
    }

    AppResponse::Success("Images".to_owned(), crate::enums::SuccessActions::Delete)
}

pub fn crud_routes(state: AppState) -> Router<AppState> {
    Router::new().nest(
        "/assets",
        Router::new()
            .merge(
                Router::new()
                    // routes must end with :id for middleware use
                    .route("/update/:id", post(update_asset))
                    .route("/:project_id/:image_type/:id", delete(delete_asset))
                    .layer(from_fn_with_state(state, permission_middleware))
                    .layer(DefaultBodyLimit::max(MAX_FILE_SIZE))
            )
            .merge(
                Router::new()
                    .route("/folder/:project_id", delete(delete_folder))
                    .route("/download/:project_id/:image_type", post(download_assets))
                    // Need the "delete" despite the method because other entities
                    // can be arkived. This is to keep a consistent URL with other
                    // entities on the UI side.
                    .route("/bulk/delete/:image_type", delete(bulk_delete_assets))
            )
    )
}
