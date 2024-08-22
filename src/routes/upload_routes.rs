use std::env;

use aws_sdk_s3::primitives::ByteStream;
use axum::{
    extract::{DefaultBodyLimit, Multipart, Path, State},
    response::IntoResponse,
    routing::post,
    Router,
};
use axum_extra::extract::CookieJar;
use reqwest::StatusCode;
use uuid::Uuid;

use crate::{
    enums::ImageType,
    state::models::AppState,
    utils::{auth_utils::check_auth, db_utils::get_client, image_utils::encode_lossy_webp},
    MAX_FILE_SIZE,
};
async fn upload_image(
    cookie_jar: CookieJar,
    State(state): State<AppState>,
    Path((project_id, image_type)): Path<(Uuid, ImageType)>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    let claims = check_auth(cookie_jar, &state.reqwest_client, state.auth_service_url).await;

    if claims.is_err() {
        return claims.err().unwrap();
    }

    let claims = claims.unwrap().claims;

    if claims.is_none() {
        return (StatusCode::UNAUTHORIZED, "UNAUTHORIZED".to_string());
    }

    let claims = claims.unwrap();

    let mut errors: Vec<String> = vec![];

    let client = get_client(&state.pool).await;

    if client.is_err() {
        return client.err().unwrap();
    }
    let client = client.unwrap();

    while let Some(field) = multipart.next_field().await.unwrap() {
        let name = field.name().unwrap_or("unnamed").to_string();
        let data = field.bytes().await;

        if name == "unnamed" {
            continue;
        }

        if data.is_err() {
            errors.push(name);
            println!("ERROR GETTING FILE DATA - {}", data.err().unwrap());
            continue;
        }

        let id = Uuid::new_v4();
        let data = data.unwrap().to_vec();

        let img_data = image::load_from_memory(&data);

        if img_data.is_err() {
            println!("{}", img_data.err().unwrap());
            continue;
        }

        let lossy = encode_lossy_webp(img_data.unwrap());

        let upload = state
            .client
            .put_object()
            .bucket(&state.bucket)
            .key(format!(
                "assets/{}/{}/{}.webp",
                &project_id, &image_type, &id
            ))
            .body(ByteStream::from(lossy))
            .acl(aws_sdk_s3::types::ObjectCannedAcl::Private)
            .content_type("image/webp")
            .cache_control("max-age=600")
            .send()
            .await;

        if upload.is_ok() {
            let res = client.query(
                "INSERT INTO images (id, title, project_id, type, owner_id) VALUES ($1, $2, $3, $4, $5);",
                &[&id, &name, &project_id, &image_type, &claims.user_id]
            ).await;

            if res.is_err() {
                println!("{}", res.err().unwrap());

                let del_res = &state
                    .client
                    .delete_object()
                    .bucket(&state.bucket)
                    .key(format!(
                        "assets/{}/{}/{}.webp",
                        &project_id, &image_type, &id
                    ))
                    .send()
                    .await;

                if del_res.is_err() {
                    println!("{}", del_res.as_ref().err().unwrap());
                    todo!("USE TRACING");
                }
                errors.push(name);
                continue;
            }
        } else {
            println!("{}", upload.err().unwrap());
            errors.push(name);
            continue;
        }
    }
    println!("{:?}", errors);
    return (StatusCode::OK, "Stagod".to_string());
}

async fn upload_user_avatar(
    cookie_jar: CookieJar,
    State(state): State<AppState>,

    mut multipart: Multipart,
) -> impl IntoResponse {
    let claims = check_auth(cookie_jar, &state.reqwest_client, state.auth_service_url).await;

    if claims.is_err() {
        return claims.err().unwrap();
    }

    let claims = claims.unwrap().claims;

    if claims.is_none() {
        return (StatusCode::UNAUTHORIZED, "UNAUTHORIZED".to_string());
    }

    let claims = claims.unwrap();

    let client = get_client(&state.pool).await;

    if client.is_err() {
        return client.err().unwrap();
    }
    let client = client.unwrap();

    let user = client
        .query_one(
            "SELECT users.id, users.image FROM users WHERE users.id = $1;",
            &[&claims.user_id],
        )
        .await;

    if user.is_err() {
        return (StatusCode::UNAUTHORIZED, "UNAUTHORIZED".to_string());
    }
    let user = user.unwrap();
    let user_id: Uuid = user.get("id");
    let user_image: Option<String> = user.get("image");

    match user_image {
        Some(img) => {
            let key = img.split("/").last().unwrap();
            let _ = &state
                .client
                .delete_object()
                .bucket(&state.bucket)
                .key(key)
                .send()
                .await;
        }
        None => {}
    }

    while let Some(field) = multipart.next_field().await.unwrap() {
        let name = field.name().unwrap_or("unnamed").to_string();
        let data = field.bytes().await;

        if name == "unnamed" {
            continue;
        }

        if data.is_err() {
            println!("ERROR GETTING FILE DATA - {}", data.err().unwrap());
            continue;
        }

        let id = Uuid::new_v4();
        let data = data.unwrap().to_vec();

        let img_data = image::load_from_memory(&data);

        if img_data.is_err() {
            println!("{}", img_data.err().unwrap());
            continue;
        }

        let lossy = encode_lossy_webp(img_data.unwrap());

        let do_spaces_name = env::var("DO_SPACES_NAME").expect("NO DO NAME");
        let do_spaces_endpoint = env::var("DO_SPACES_ENDPOINT")
            .expect("NO DO ENDPOINT")
            .replace("https://", "");
        let key = format!("assets/avatars/{}-{}.webp", &user_id, &id);

        let upload = state
            .client
            .put_object()
            .bucket(&state.bucket)
            .key(&key)
            .body(ByteStream::from(lossy))
            .acl(aws_sdk_s3::types::ObjectCannedAcl::PublicRead)
            .content_type("image/webp")
            .cache_control("max-age=600")
            .send()
            .await;

        if upload.is_ok() {
            let new_url = format!("https://{}.{}/{}", do_spaces_name, do_spaces_endpoint, &key);
            let res = client
                .query(
                    "UPDATE users SET image = $1 WHERE users.id = $2",
                    &[&new_url, &claims.user_id],
                )
                .await;

            if res.is_err() {
                println!("{}", res.err().unwrap());

                let del_res = &state
                    .client
                    .delete_object()
                    .bucket(&state.bucket)
                    .key(&key)
                    .send()
                    .await;

                if del_res.is_err() {
                    println!("{}", del_res.as_ref().err().unwrap());
                    todo!("USE TRACING");
                }

                continue;
            }
        } else {
            println!("{}", upload.err().unwrap());
            continue;
        }
    }
    return (StatusCode::OK, "Stagod".to_string());
}

pub fn upload_routes() -> Router<AppState> {
    Router::new().nest(
        "/upload",
        Router::new()
            .route("/:project_id/:image_type", post(upload_image))
            .route("/users/avatar", post(upload_user_avatar))
            .layer(DefaultBodyLimit::max(MAX_FILE_SIZE)),
    )
}
