use std::fmt::Display;

use axum::{ response::{ IntoResponse, Response }, Json };
use postgres_types::{ FromSql, ToSql };
use reqwest::StatusCode;
use serde::{ Deserialize, Serialize };
#[derive(Deserialize, Debug, ToSql, FromSql)]
#[serde(rename_all = "snake_case")]
#[postgres(name = "ImageType")]
pub enum ImageType {
    #[postgres(name = "images")]
    Images,
    #[postgres(name = "map_images")]
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

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SupportedImageType {
    Jpeg,
    Jpg,
    Png,
    Webp,
    Avif,
    Gif,
}

#[derive(Debug)]
pub enum SuccessActions {
    Create,
    Update,
    Delete,
    Upload,
}

impl Display for SuccessActions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let output = match self {
            &SuccessActions::Create => "created",
            &SuccessActions::Update => "updated",
            &SuccessActions::Delete => "deleted",
            &SuccessActions::Upload => "uploaded",
        };
        write!(f, "{}", output)
    }
}

#[derive(Debug)]
pub enum AppResponse {
    Success(String, SuccessActions),
    Error(String),
    Auth,
    Unauthorized,
}

impl IntoResponse for AppResponse {
    fn into_response(self) -> Response {
        #[derive(Serialize)]
        struct ResponsePayload {
            ok: bool,
            message: String,
            role_access: bool,
        }

        let (status, res) = match self {
            AppResponse::Success(entity, action) => {
                (
                    StatusCode::OK,
                    Json(ResponsePayload {
                        ok: true,
                        message: format!("{} successfully {}.", entity, action),
                        role_access: true,
                    }),
                )
            }
            AppResponse::Error(err) => {
                tracing::error!("ERROR - {}", err);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ResponsePayload {
                        ok: false,
                        message: "There was an error with your request.".to_owned(),
                        role_access: true,
                    }),
                )
            }
            AppResponse::Auth => {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ResponsePayload {
                        ok: false,
                        message: "You do not have permission to perform this action.".to_owned(),
                        role_access: false,
                    }),
                )
            }
            AppResponse::Unauthorized => {
                (
                    StatusCode::UNAUTHORIZED,
                    Json(ResponsePayload {
                        ok: false,
                        message: "UNAUTHORIZED".to_owned(),
                        role_access: false,
                    }),
                )
            }
        };

        (status, res).into_response()
    }
}
