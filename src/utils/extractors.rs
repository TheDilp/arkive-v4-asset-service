use axum::{
    async_trait,
    extract::{ rejection::PathRejection, FromRequestParts },
    http::request::Parts,
};
use serde::de::DeserializeOwned;

use crate::enums::AppResponse;

pub struct ExtractPath<T>(pub T);

#[async_trait]
impl<S, T> FromRequestParts<S>
    for ExtractPath<T>
    where
        // these trait bounds are copied from `impl FromRequest for axum::extract::path::Path`
        T: DeserializeOwned + Send,
        S: Send + Sync
{
    type Rejection = AppResponse;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        match axum::extract::Path::<T>::from_request_parts(parts, state).await {
            Ok(value) => Ok(Self(value.0)),
            Err(rejection) => {
                let err_res = match rejection {
                    PathRejection::FailedToDeserializePathParams(inner) => {
                        let kind = inner.into_kind();

                        AppResponse::Error(format!("PATH ERROR - {}", kind.to_string()))
                    }
                    PathRejection::MissingPathParams(error) => {
                        AppResponse::Error(format!("PATH ERROR - {}", error.to_string()))
                    }

                    _ => { AppResponse::Error(format!("Unhandled path rejection: {rejection}")) }
                };

                return Err(err_res);
            }
        }
    }
}
