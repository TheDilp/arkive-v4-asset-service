use std::{ collections::HashMap, env, fmt::Display, io::Cursor, time::Duration };

use aws_config::{ BehaviorVersion, Region };
use aws_sdk_s3::{ config::Credentials, presigning::PresigningConfig, Client };
use axum::{
    extract::{ Path, Query, State },
    http::HeaderValue,
    response::IntoResponse,
    routing::get,
    Router,
};
use axum_extra::extract::{ cookie::Cookie, CookieJar };
use axum_macros::debug_handler;
use image::{ imageops::FilterType, load_from_memory };
use reqwest::{
    header::{ CACHE_CONTROL, CONTENT_TYPE },
    Client as ReqwestClient,
    Method,
    StatusCode,
};
use serde::Deserialize;
use tokio::net::TcpListener;
use tower_http::cors::{ AllowOrigin, CorsLayer };
use uuid::Uuid;

const PRESIGN_DURATION: Duration = Duration::from_secs(300);

#[derive(Clone)]
struct AppState {
    client: Client,
    bucket: String,
    reqwest_client: ReqwestClient,
    auth_service_url: String,
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
    Path((project_id, image_type, image_id)): Path<(Uuid, ImageType, Uuid)>
) -> impl IntoResponse {
    let access_token = cookie_jar.get("access").unwrap_or(&Cookie::new("access", "")).to_string();
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
            Vec::new(),
        );
    }

    let command = state.client
        .get_object()
        .bucket(&state.bucket)
        .key(format!("assets/{}/{}/{}.webp", &project_id, &image_type, &image_id))
        .presigned(PresigningConfig::expires_in(PRESIGN_DURATION).unwrap()).await
        .unwrap();

    let url = command.uri();

    let img_bytes = reqwest
        ::get(url.replace("nyc3.", "nyc3.cdn.")).await
        .unwrap()
        .bytes().await
        .unwrap();

    let img = load_from_memory(&img_bytes).unwrap();

    if query.width.is_some() && query.height.is_some() {
        let resized = img.resize(35, 35, FilterType::Nearest);

        let mut bytes: Vec<u8> = Vec::new();
        resized.write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png).unwrap();

        return (
            StatusCode::OK,
            [
                (CONTENT_TYPE, HeaderValue::from_str("image/webp").unwrap()),
                (CACHE_CONTROL, HeaderValue::from_str("max-age=600").unwrap()),
            ],
            bytes,
        );
    }

    let mut bytes: Vec<u8> = Vec::new();
    img.write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png).unwrap();

    return (
        StatusCode::OK,
        [
            (CONTENT_TYPE, HeaderValue::from_str("image/webp").unwrap()),
            (CACHE_CONTROL, HeaderValue::from_str("max-age=600").unwrap()),
        ],
        bytes,
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

    let creds = Credentials::new(access_key_id, secret_access_key, None, None, "");
    let reqwest_client = reqwest::Client::new();
    let config = aws_sdk_s3::config::Builder
        ::new()
        .behavior_version(BehaviorVersion::latest())
        .force_path_style(false)
        .region(Region::new("us-east-1"))
        .endpoint_url(endpoint_url)
        .credentials_provider(creds)
        .build();

    let client = aws_sdk_s3::Client::from_conf(config);

    let listener = TcpListener::bind("[::]:5184").await.unwrap();

    let origins = AllowOrigin::list([editor_client.parse().unwrap()]);

    let cors = CorsLayer::new().allow_methods([Method::GET]).allow_origin(origins);

    let app = Router::new()
        .route("/:project_id/:image_type/:image_id", get(get_thumbnail))
        .with_state(AppState { client, bucket, reqwest_client, auth_service_url })
        .layer(cors);

    axum::serve(listener, app).await.unwrap();
}
