use deadpool_postgres::{ Object, Pool };

use crate::enums::AppResponse;
pub async fn get_client(pool: &Pool) -> Result<Object, AppResponse> {
    let client = pool.get().await;

    if client.is_err() {
        return Err(AppResponse::Error(client.err().unwrap().to_string()));
    }

    Ok(client.unwrap())
}
