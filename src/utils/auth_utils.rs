use std::collections::HashMap;

use axum_extra::extract::{ cookie::Cookie, CookieJar };
use reqwest::{ header::CONTENT_TYPE, Client, StatusCode };

use crate::{
    enums::AppResponse,
    state::models::{ AppState, PermissionUpdateType, VerifyJWTResponse },
};

use super::db_utils::get_client;

pub async fn check_auth(
    cookie_jar: CookieJar,
    client: &Client,
    auth_service_url: String
) -> Result<VerifyJWTResponse, (StatusCode, String)> {
    let access_token = cookie_jar.get("access").unwrap_or(&Cookie::new("access", "")).to_string();
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
        .send().await
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

pub async fn insert_permissions(
    permissions: Option<String>,
    state: &AppState
) -> Result<(), AppResponse> {
    if permissions.is_some() {
        let permissions = permissions.unwrap();

        let permissions: Vec<PermissionUpdateType> = serde_json::from_str(&permissions).unwrap();

        for perm in permissions {
            if perm.role_id.is_some() && perm.permission_id.is_none() && perm.user_id.is_none() {
                let client = get_client(&state.pool).await;
                if client.is_err() {
                    return Err(client.err().unwrap());
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
                            return Err(
                                AppResponse::Error(transaction_result.err().unwrap().to_string())
                            );
                        }
                    } else {
                        return Err(AppResponse::Error(insert_res.err().unwrap().to_string()));
                    }
                } else {
                    return Err(AppResponse::Error(del_res.err().unwrap().to_string()));
                }
            } else if perm.permission_id.is_some() && perm.user_id.is_some() {
                let client = get_client(&state.pool).await;
                if client.is_err() {
                    return Err(client.err().unwrap());
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
                            return Err(
                                AppResponse::Error(transaction_result.err().unwrap().to_string())
                            );
                        }
                    } else {
                        return Err(AppResponse::Error(insert_res.err().unwrap().to_string()));
                    }
                } else {
                    return Err(AppResponse::Error(del_res.err().unwrap().to_string()));
                }
            }
        }
    }
    return Ok(());
}
