use aws_sdk_s3::primitives::ByteStream;
use axum::{
    extract::{ DefaultBodyLimit, Multipart, Path, State },
    response::IntoResponse,
    routing::post,
    Router,
};
use axum_extra::extract::CookieJar;
use image::DynamicImage;
use reqwest::StatusCode;
use uuid::Uuid;

fn encode_lossy_webp(img: DynamicImage) -> Vec<u8> {
    let img = img.to_rgba8();
    let (width, height) = img.dimensions();
    webp::Encoder::new(&*img, webp::PixelLayout::Rgba, width, height).encode(1.0).to_vec()
}

use crate::{
    enums::ImageType,
    state::models::AppState,
    utils::{ auth_utils::check_auth, db_utils::get_client },
    MAX_FILE_SIZE,
};
async fn upload_image(
    cookie_jar: CookieJar,
    State(state): State<AppState>,
    Path((project_id, image_type)): Path<(Uuid, ImageType)>,
    mut multipart: Multipart
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

        let upload = state.client
            .put_object()
            .bucket(&state.bucket)
            .key(format!("assets/{}/{}/{}.webp", &project_id, &image_type, &id))
            .body(ByteStream::from(lossy))
            .acl(aws_sdk_s3::types::ObjectCannedAcl::Private)
            .content_type("image/webp")
            .cache_control("max-age=600")
            .send().await;

        if upload.is_ok() {
            let res = client.query(
                "INSERT INTO images (id, title, project_id, type, owner_id) VALUES ($1, $2, $3, $4, $5);",
                &[&id, &name, &project_id, &image_type, &claims.user_id]
            ).await;

            if res.is_err() {
                println!("{}", res.err().unwrap());

                let del_res = &state.client
                    .delete_object()
                    .bucket(&state.bucket)
                    .key(format!("assets/{}/{}/{}.webp", &project_id, &image_type, &id))
                    .send().await;

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

// async fn upload_user_image(
//     cookie_jar: CookieJar,
//     State(state): State<AppState>,
//     Path(id): Path<Uuid>,
//     mut multipart: Multipart
// ) -> impl IntoResponse {
//     let claims = check_auth(cookie_jar, &state.reqwest_client, state.auth_service_url).await;

//     if claims.is_err() {
//         return claims.err().unwrap();
//     }

//     let claims = claims.unwrap().claims;

//     if claims.is_none() {
//         return (StatusCode::UNAUTHORIZED, "UNAUTHORIZED".to_string());
//     }

//     let claims = claims.unwrap();

//     let mut errors: Vec<String> = vec![];

//     let client = get_client(&state.pool).await;

//     if client.is_err() {
//         return client.err().unwrap();
//     }
//     let client = client.unwrap();

//     while let Some(field) = multipart.next_field().await.unwrap() {
//         let name = field.name().or(Some("unnamed")).unwrap().to_string();
//         println!("{}", name);
//         if name == "unnamed" {
//             continue;
//         }

//         let data = field.bytes().await;

//         if data.is_err() {
//             errors.push(name);
//             println!("ERROR GETTING FILE DATA - {}", data.err().unwrap());
//             continue;
//         }
//         let data = data.unwrap();
//         let id = Uuid::new_v4();

//         let command = state.client
//             .put_object()
//             .bucket(&state.bucket)
//             .key(format!("assets/avatars/{}-{}.webp", &project_id, &image_type, &id))
//             .acl(aws_sdk_s3::types::ObjectCannedAcl::Private)
//             .content_type("image/webp")
//             .cache_control("max-age=600")
//             // default duration is 5 mins
//             // dividing by 5 gives 1 min
//             .presigned(PresigningConfig::expires_in(PRESIGN_DURATION / 5).unwrap()).await
//             .unwrap();

//         let url = command.uri();

//         let mut headers = HeaderMap::new();

//         headers.append(CONTENT_TYPE, HeaderValue::from_str("image/webp").unwrap());
//         headers.append(CACHE_CONTROL, HeaderValue::from_str("max-age=600").unwrap());
//         headers.append("x-amz-acl", HeaderValue::from_str("private").unwrap());

//         let upload = &state.reqwest_client.put(url).body(data).headers(headers).send().await;

//         if upload.is_ok() {
//             let res = client.query(
//                 "INSERT INTO images (id, title, project_id, type, owner_id) VALUES ($1, $2, $3, $4, $5);",
//                 &[&id, &name, &project_id, &image_type, &claims.user_id]
//             ).await;

//             if res.is_err() {
//                 println!("{}", res.err().unwrap());

//                 let del_res = &state.client
//                     .delete_object()
//                     .bucket(&state.bucket)
//                     .key(format!("assets/{}/{}/{}.webp", &project_id, &image_type, &id))
//                     .send().await;

//                 if del_res.is_err() {
//                     println!("{}", del_res.as_ref().err().unwrap());
//                     todo!("USE TRACING");
//                 }
//                 errors.push(name);
//                 continue;
//             }
//         } else {
//             println!("{}", upload.as_ref().err().unwrap());
//             errors.push(name);
//             continue;
//         }
//     }
//     println!("{:?}", errors);
//     return (StatusCode::OK, "Stagod".to_string());
// }

pub fn upload_routes() -> Router<AppState> {
    Router::new().nest(
        "/upload",
        Router::new()
            .route("/:project_id/:image_type", post(upload_image))
            .layer(DefaultBodyLimit::max(MAX_FILE_SIZE))
    )
}
