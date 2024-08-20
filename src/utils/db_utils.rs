use axum::http::StatusCode;
use deadpool_postgres::{Object, Pool};
pub async fn get_client(pool: &Pool) -> Result<Object, (StatusCode, String)> {
    let client = pool.get().await;

    if client.is_err() {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            "UNAUTHORIZED".to_string(),
        ));
    }

    Ok(client.unwrap())
}
