use std::{ env, str::FromStr, time::Duration };

use aws_config::{ BehaviorVersion, Region };
use aws_sdk_s3::config::Credentials;
use axum::{ extract::{ MatchedPath, Request }, http::HeaderName, Router };
use deadpool_postgres::{ Config as DeadPoolConfig, ManagerConfig };
use reqwest::{ header::CONTENT_TYPE, Method };
use routes::{
    crud_routes::crud_routes,
    thumbnail_routes::thumbnail_routes,
    upload_routes::upload_routes,
};
use state::models::AppState;
use tokio::net::TcpListener;
use tokio_postgres::NoTls;
use tower_http::{ cors::{ AllowOrigin, CorsLayer }, trace::TraceLayer };

mod enums;
mod routes;
mod state;
mod utils;

const PRESIGN_DURATION: Duration = Duration::from_secs(1800); // 30 mins
const MAX_FILE_SIZE: usize = 20_000_000;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    dotenv::dotenv().ok();

    let endpoint_url = env::var("DO_SPACES_ENDPOINT").unwrap();
    let access_key_id = env::var("DO_SPACES_KEY").unwrap();
    let secret_access_key = env::var("DO_SPACES_SECRET").unwrap();
    let bucket = env::var("DO_SPACES_NAME").unwrap();

    let editor_client = env::var("EDITOR_CLIENT_URL").unwrap();
    let wiki_client = env::var("WIKI_CLIENT_URL").unwrap();
    let gateway_client = env::var("GATEWAY_CLIENT_URL").unwrap();
    let dyce_client = env::var("DYCE_CLIENT_URL").unwrap();

    let auth_service_url = env::var("AUTH_SERVICE_URL").unwrap();
    let thumbnail_service_url = env::var("THUMBNAIL_SERVICE").unwrap();

    let thumbnail_secret = env::var("THUMBNAIL_SECRET").unwrap();

    let database_url = env::var("DATABASE_URL").expect("NO DB URL CONFIGURED");

    let mut cfg = DeadPoolConfig::new();
    cfg.url = Some(database_url);

    cfg.manager = Some(ManagerConfig {
        recycling_method: deadpool_postgres::RecyclingMethod::Fast,
    });
    let pool = cfg.create_pool(Some(deadpool_postgres::Runtime::Tokio1), NoTls).unwrap();

    let port = env::var("PORT").unwrap();

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

    let listener = TcpListener::bind(format!("[::]:{}", port)).await.unwrap();

    let origins = AllowOrigin::list([
        editor_client.parse().unwrap(),
        gateway_client.parse().unwrap(),
        wiki_client.parse().unwrap(),
        dyce_client.parse().unwrap(),
    ]);

    let cors = CorsLayer::new()
        .allow_methods([Method::GET, Method::POST, Method::DELETE, Method::OPTIONS])
        .allow_credentials(true)
        .allow_headers([HeaderName::from_str("module").unwrap(), CONTENT_TYPE])
        .allow_origin(origins);

    let state = AppState {
        client,
        bucket,
        reqwest_client,
        auth_service_url,
        thumbnail_secret,
        thumbnail_service_url,
        pool,
    };

    let app = Router::new()
        .merge(crud_routes(state.clone()))
        .merge(upload_routes())
        .merge(thumbnail_routes())
        .layer(cors)
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|req: &Request| {
                    let method = req.method();
                    let uri = req.uri();

                    let matched_path = req
                        .extensions()
                        .get::<MatchedPath>()
                        .map(|matched_path| matched_path.as_str());

                    tracing::error_span!("Request error at", %method, %uri, matched_path)
                })
                .on_failure(())
        )
        .with_state(state);

    println!("RUNNING ON PORT {} 🚀", port);

    axum::serve(listener, app).await.unwrap();
}
