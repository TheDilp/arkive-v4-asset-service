// use std::collections::HashMap;

use aws_sdk_s3::Client;
use deadpool_postgres::Pool;
use reqwest::Client as ReqwestClient;
use serde::Deserialize;
use uuid::Uuid;

#[derive(Clone)]
pub struct AppState {
    pub client: Client,
    pub bucket: String,
    pub reqwest_client: ReqwestClient,
    pub auth_service_url: String,
    pub thumbnail_secret: String,
    pub thumbnail_service_url: String,
    pub discord_service_url: String,
    pub discord_service_api_key: String,
    pub pool: Pool,
}

#[derive(Debug, Deserialize)]
pub struct Claims {
    pub user_id: Uuid,
    pub project_id: Uuid,
}

#[derive(Deserialize)]
pub struct VerifyJWTResponse {
    pub claims: Option<Claims>,
}

#[derive(Deserialize)]
pub struct PermissionCheckResponse {
    pub is_project_owner: bool,
    // pub all_permissions: HashMap<String, bool>,
    // pub role_access: bool,
    pub role_id: Option<Uuid>,
    pub permission_id: Option<Uuid>,
}
