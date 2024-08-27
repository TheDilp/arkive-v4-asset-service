use std::collections::HashMap;

use aws_sdk_s3::presigning::PresigningConfig;
use axum::{
    extract::{ Query, Request, State },
    http::HeaderValue,
    response::IntoResponse,
    routing::get,
    Router,
};
use axum_extra::extract::{ cookie::Cookie, CookieJar };
use axum_macros::debug_handler;
use base64::prelude::*;
use hmac::{ Hmac, Mac };
use reqwest::{ header::{ CACHE_CONTROL, CONTENT_TYPE }, StatusCode };
use serde::Deserialize;
use sha2::Sha512;
use url::Url;
use uuid::Uuid;

use crate::{
    enums::ImageType,
    state::models::AppState,
    utils::extractors::ExtractPath,
    PRESIGN_DURATION,
};

type HmacSha512 = Hmac<Sha512>;

#[derive(Deserialize)]
struct ThumbnailDimensions {
    width: Option<usize>,
    height: Option<usize>,
}

#[debug_handler]
async fn get_thumbnail(
    State(state): State<AppState>,
    cookie_jar: CookieJar,
    query: Query<ThumbnailDimensions>,
    ExtractPath((project_id, image_type, image_id)): ExtractPath<(Uuid, ImageType, Uuid)>,
    request: Request
) -> impl IntoResponse {
    let headers = request.headers();

    let url = Url::parse(&state.discord_service_url).unwrap();
    let domain = url.domain();
    let discord_service_domain = url.domain().unwrap();
    if let Some(domain_string) = domain {
        if domain_string.to_string() != discord_service_domain {
            let access_token = cookie_jar
                .get("access")
                .unwrap_or(&Cookie::new("access", ""))
                .to_string();
            let refresh_token = cookie_jar
                .get("refresh")
                .unwrap_or(&Cookie::new("refresh", ""))
                .to_string();

            let mut map = HashMap::new();

            map.insert("access", access_token);
            map.insert("refresh", refresh_token);

            let res = state.reqwest_client
                .post(format!("{}/verify", &state.auth_service_url))
                .header(CONTENT_TYPE, "application/json")
                .json(&map)
                .send().await
                .unwrap();

            if res.status() != StatusCode::OK {
                return (
                    StatusCode::UNAUTHORIZED,
                    [
                        (CONTENT_TYPE, HeaderValue::from_str("image/webp").unwrap()),
                        (CACHE_CONTROL, HeaderValue::from_str("max-age=0").unwrap()),
                    ],
                    "".to_string(),
                );
            }
        } else {
            let api_key_header = headers.get("X-API-KEY");

            if let Some(api_key) = api_key_header {
                if api_key.to_str().unwrap().to_owned() != state.discord_service_api_key {
                    return (
                        StatusCode::UNAUTHORIZED,
                        [
                            (CONTENT_TYPE, HeaderValue::from_str("image/webp").unwrap()),
                            (CACHE_CONTROL, HeaderValue::from_str("max-age=0").unwrap()),
                        ],
                        "".to_string(),
                    );
                }
            }
        }
    }

    if query.width.is_some() && query.height.is_some() {
        let mut hmac = HmacSha512::new_from_slice(&state.thumbnail_secret.as_bytes()).unwrap();
        let sized_url = format!(
            "{}x{}/assets/{}/{}/{}.webp",
            query.width.unwrap(),
            query.height.unwrap(),
            &project_id,
            &image_type,
            &image_id
        );
        hmac.update(&sized_url.as_bytes());

        let res = hmac.finalize().into_bytes();

        let base_64 = BASE64_STANDARD.encode(res).replace('+', "-").replace('/', "_");

        let url = format!("{}/{}/{}", &state.thumbnail_service_url, &base_64, &sized_url);

        return (
            StatusCode::OK,
            [
                (CONTENT_TYPE, HeaderValue::from_str("text/plain").unwrap()),
                (CACHE_CONTROL, HeaderValue::from_str("max-age=3600").unwrap()),
            ],
            url.to_string(),
        );
    }

    let command = state.client
        .get_object()
        .bucket(&state.bucket)
        .key(format!("assets/{}/{}/{}.webp", &project_id, &image_type, &image_id))
        .presigned(PresigningConfig::expires_in(PRESIGN_DURATION).unwrap()).await
        .unwrap();

    let url = command.uri();

    return (
        StatusCode::OK,
        [
            (CONTENT_TYPE, HeaderValue::from_str("text/plain").unwrap()),
            (CACHE_CONTROL, HeaderValue::from_str("max-age=3600").unwrap()),
        ],
        url.to_string(),
    );
}

pub fn thumbnail_routes() -> Router<AppState> {
    Router::new().route("/:project_id/:image_type/:image_id", get(get_thumbnail))
}
