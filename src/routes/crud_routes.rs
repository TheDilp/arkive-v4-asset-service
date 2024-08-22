use aws_sdk_s3::primitives::ByteStream;
use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, Path, State},
    response::IntoResponse,
    routing::post,
    Router,
};
use axum_extra::extract::CookieJar;
use axum_typed_multipart::{FieldData, TryFromMultipart, TypedMultipart};
use reqwest::StatusCode;
use uuid::Uuid;

use crate::{
    enums::ImageType,
    state::models::AppState,
    utils::{auth_utils::check_auth, db_utils::get_client, image_utils::encode_lossy_webp},
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
    cookie_jar: CookieJar,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    TypedMultipart(UpdatePayload {
        title,
        owner_id,
        file,
    }): TypedMultipart<UpdatePayload>,
) -> impl IntoResponse {
    let claims = check_auth(cookie_jar, &state.reqwest_client, state.auth_service_url).await;

    if claims.is_err() {
        return claims.err().unwrap();
    }

    let claims = claims.unwrap().claims;

    if claims.is_none() {
        return (StatusCode::UNAUTHORIZED, "UNAUTHORIZED".to_string());
    }

    let claims = claims.unwrap();

    let client = get_client(&state.pool).await;

    if client.is_err() {
        return client.err().unwrap();
    }
    let client = client.unwrap();
    if title.is_some() || owner_id.is_some() {
        if title.is_some() && owner_id.is_some() {
            let res = client
                .query(
                    "title = $1, owner_id = $2 WHERE id = $3",
                    &[&title.unwrap(), &owner_id.unwrap(), &id],
                )
                .await;

            if res.is_err() {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "There was an error with your request".to_string(),
                );
            }
        } else if title.is_some() && owner_id.is_none() {
            let res = client
                .query("title = $1 WHERE id = $2", &[&title.unwrap(), &id])
                .await;

            if res.is_err() {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "There was an error with your request".to_string(),
                );
            }
        } else if title.is_none() && owner_id.is_some() {
            let res = client
                .query("owner_id = $1 WHERE id = $2", &[&owner_id.unwrap(), &id])
                .await;

            if res.is_err() {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "There was an error with your request".to_string(),
                );
            }
        }
    }

    if file.is_some() {
        let current_image = client
            .query_one("SELECT project_id, type FROM images WHERE id = $1;", &[&id])
            .await;

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

        let upload = state
            .client
            .put_object()
            .bucket(&state.bucket)
            .key(format!(
                "assets/{}/{}/{}.webp",
                &project_id, &image_type, &id
            ))
            .body(ByteStream::from(lossy))
            .acl(aws_sdk_s3::types::ObjectCannedAcl::Private)
            .content_type("image/webp")
            .cache_control("max-age=600")
            .send()
            .await;

        if upload.is_err() {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "There was an error with your request".to_string(),
            );
        }
    }

    return (StatusCode::OK, "Stagod".to_string());
}

pub fn crud_routes() -> Router<AppState> {
    Router::new().nest(
        "/upload",
        Router::new()
            .route("/update/:id", post(update_asset))
            .layer(DefaultBodyLimit::max(MAX_FILE_SIZE)),
    )
}
