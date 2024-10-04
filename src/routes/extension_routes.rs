use std::str::FromStr;

use aws_sdk_s3::primitives::ByteStream;
use axum::{
    extract::{ Multipart, State },
    http::{ HeaderMap, HeaderName },
    response::IntoResponse,
    routing::post,
    Router,
};
use reqwest::Method;
use tower_http::cors::{ AllowOrigin, CorsLayer };
use uuid::Uuid;

use crate::{
    enums::{ AppResponse, ImageType },
    state::models::AppState,
    utils::{ db_utils::get_client, image_utils::encode_lossy_webp },
};

async fn upload(
    State(state): State<AppState>,
    headers: HeaderMap,
    mut multipart: Multipart
) -> impl IntoResponse {
    let api_key = headers.get("x-api-key");
    if api_key.is_none() {
        return AppResponse::Unauthorized;
    }
    let api_key = api_key.unwrap().to_str().unwrap().to_string();
    let client = get_client(&state.pool).await;

    if client.is_err() {
        return client.err().unwrap();
    }

    let client = client.unwrap();

    let is_api_key_valid = client.query_one(
        "SELECT id, owner_id FROM projects WHERE api_key = $1;",
        &[&api_key]
    ).await;

    if is_api_key_valid.is_err() {
        return AppResponse::Unauthorized;
    }

    let data = is_api_key_valid.unwrap();

    let project_id: Uuid = data.get("id");
    let user_id: Uuid = data.get("owner_id");

    while let Some(field) = multipart.next_field().await.unwrap() {
        let name = field.name().unwrap_or("unnamed").to_string();
        let data = field.bytes().await;

        if name == "unnamed" {
            continue;
        }

        if data.is_err() {
            return AppResponse::Error(
                format!("ERROR GETTING FILE DATA EXTENSION ROUTE - {}", data.err().unwrap())
            );
        }

        let id = Uuid::new_v4();
        let data = data.unwrap().to_vec();

        let img_data = image::load_from_memory(&data);

        if img_data.is_err() {
            return AppResponse::Error(format!("{}", img_data.err().unwrap()));
        }

        let lossy = encode_lossy_webp(img_data.unwrap());

        let upload = state.client
            .put_object()
            .bucket(&state.bucket)
            .key(format!("assets/{}/{}/{}.webp", &project_id, &ImageType::Images, &id))
            .body(ByteStream::from(lossy))
            .acl(aws_sdk_s3::types::ObjectCannedAcl::Private)
            .content_type("image/webp")
            .cache_control("max-age=600")
            .send().await;

        if upload.is_ok() {
            let res = client.query(
                "INSERT INTO images (id, title, project_id, type, owner_id) VALUES ($1, $2, $3, $4, $5);",
                &[&id, &name, &project_id, &ImageType::Images, &user_id]
            ).await;

            if res.is_err() {
                let del_res = &state.client
                    .delete_object()
                    .bucket(&state.bucket)
                    .key(format!("assets/{}/{}/{}.webp", &project_id, &ImageType::Images, &id))
                    .send().await;

                if del_res.is_err() {
                    tracing::error!("{}", del_res.as_ref().err().unwrap());
                }
                return AppResponse::Error(format!("{}", res.err().unwrap()));
            }
        } else {
            return AppResponse::Error(format!("{}", upload.err().unwrap()));
        }
    }

    return AppResponse::Success("".to_string(), crate::enums::SuccessActions::Upload);
}

pub fn extension_routes() -> Router<AppState> {
    let extension_cors = CorsLayer::new()
        .allow_methods([Method::POST, Method::OPTIONS])
        .allow_headers([HeaderName::from_str("x-api-key").unwrap()])
        .allow_origin(AllowOrigin::any());
    Router::new().nest(
        "/extension",
        Router::new().route("/upload", post(upload)).layer(extension_cors)
    )
}
