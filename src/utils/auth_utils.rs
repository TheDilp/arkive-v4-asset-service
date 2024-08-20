use std::collections::HashMap;

use axum_extra::extract::{cookie::Cookie, CookieJar};
use reqwest::{header::CONTENT_TYPE, Client, StatusCode};

use crate::state::models::VerifyJWTResponse;

pub async fn check_auth(
    cookie_jar: CookieJar,
    client: &Client,
    auth_service_url: String,
) -> Result<VerifyJWTResponse, (StatusCode, String)> {
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

    let res = client
        .post(format!("{}/verify", &auth_service_url))
        .header(CONTENT_TYPE, "application/json")
        .json(&map)
        .send()
        .await
        .unwrap();

    if res.status() == StatusCode::OK {
        let data = res.json::<VerifyJWTResponse>().await;

        if data.is_err() {
            return Err((StatusCode::UNAUTHORIZED, "UNAUTHORIZED".to_string()));
        }

        let data = data.unwrap();

        return Ok(data);
    }
    return Err((StatusCode::UNAUTHORIZED, "UNAUTHORIZED".to_string()));
}
