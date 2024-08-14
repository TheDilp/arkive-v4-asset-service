use std::{collections::HashMap, env, fmt::Display, time::Duration};

use aws_config::{BehaviorVersion, Region};
use aws_sdk_s3::{config::Credentials, presigning::PresigningConfig, Client};
use axum::{
    extract::{Path, Query, State},
    http::HeaderValue,
    response::IntoResponse,
    routing::get,
    Router,
};
use axum_extra::extract::{cookie::Cookie, CookieJar};
use axum_macros::debug_handler;
use base64::prelude::*;
use hmac::{Hmac, Mac};
use reqwest::{
    header::{CACHE_CONTROL, CONTENT_TYPE},
    Client as ReqwestClient, Method, StatusCode,
};
use serde::Deserialize;
use sha2::Sha512;
use tokio::net::TcpListener;
use tower_http::cors::{AllowOrigin, CorsLayer};
use uuid::Uuid;
const PRESIGN_DURATION: Duration = Duration::from_secs(300);
type HmacSha512 = Hmac<Sha512>;
#[derive(Clone)]
struct AppState {
    client: Client,
    bucket: String,
    reqwest_client: ReqwestClient,
    auth_service_url: String,
    thumbnail_secret: String,
    thumbnail_service_url: String,
}

#[derive(Deserialize)]
struct Dimensions {
    width: Option<usize>,
    height: Option<usize>,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum ImageType {
    Images,
    MapImages,
}

impl Display for ImageType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let output = match self {
            &ImageType::Images => "images",
            &ImageType::MapImages => "map_images",
        };
        write!(f, "{}", output)
    }
}

#[debug_handler]
async fn get_thumbnail(
    State(state): State<AppState>,
    cookie_jar: CookieJar,
    query: Query<Dimensions>,
    Path((project_id, image_type, image_id)): Path<(Uuid, ImageType, Uuid)>,
) -> impl IntoResponse {
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

    let res = state
        .reqwest_client
        .post(format!("{}/verify", &state.auth_service_url))
        .header(CONTENT_TYPE, "application/json")
        .json(&map)
        .send()
        .await
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

        let base_64 = BASE64_STANDARD
            .encode(res)
            .replace('+', "-")
            .replace('/', "_");

        let url = format!(
            "{}/{}/{}",
            &state.thumbnail_service_url, &base_64, &sized_url
        );

        return (
            StatusCode::OK,
            [
                (CONTENT_TYPE, HeaderValue::from_str("text/plain").unwrap()),
                (
                    CACHE_CONTROL,
                    HeaderValue::from_str("max-age=3600").unwrap(),
                ),
            ],
            url.to_string(),
        );
    }

    let command = state
        .client
        .get_object()
        .bucket(&state.bucket)
        .key(format!(
            "assets/{}/{}/{}.webp",
            &project_id, &image_type, &image_id
        ))
        .presigned(PresigningConfig::expires_in(PRESIGN_DURATION).unwrap())
        .await
        .unwrap();

    let url = command.uri();

    return (
        StatusCode::OK,
        [
            (CONTENT_TYPE, HeaderValue::from_str("text/plain").unwrap()),
            (
                CACHE_CONTROL,
                HeaderValue::from_str("max-age=3600").unwrap(),
            ),
        ],
        url.to_string(),
    );
}

#[tokio::main]
async fn main() {
    dotenv::dotenv().ok();

    let endpoint_url = env::var("DO_SPACES_ENDPOINT").unwrap();
    let access_key_id = env::var("DO_SPACES_KEY").unwrap();
    let secret_access_key = env::var("DO_SPACES_SECRET").unwrap();
    let bucket = env::var("DO_SPACES_NAME").unwrap();

    let editor_client = env::var("EDITOR_CLIENT").unwrap();
    let auth_service_url = env::var("AUTH_SERVICE").unwrap();
    let thumbnail_service_url = env::var("THUMBNAIL_SERVICE").unwrap();

    let thumbnail_secret = env::var("THUMBNAIL_SECRET").unwrap();

    let creds = Credentials::new(access_key_id, secret_access_key, None, None, "");
    let reqwest_client = reqwest::Client::new();
    let config = aws_sdk_s3::config::Builder::new()
        .behavior_version(BehaviorVersion::latest())
        .force_path_style(false)
        .region(Region::new("us-east-1"))
        .endpoint_url(endpoint_url)
        .credentials_provider(creds)
        .build();

    let client = aws_sdk_s3::Client::from_conf(config);

    let listener = TcpListener::bind("[::]:5184").await.unwrap();

    let origins = AllowOrigin::list([editor_client.parse().unwrap()]);

    let cors = CorsLayer::new()
        .allow_methods([Method::GET])
        .allow_origin(origins);

    let app = Router::new()
        .route("/:project_id/:image_type/:image_id", get(get_thumbnail))
        .with_state(AppState {
            client,
            bucket,
            reqwest_client,
            auth_service_url,
            thumbnail_secret,
            thumbnail_service_url,
        })
        .layer(cors);

    axum::serve(listener, app).await.unwrap();
}
