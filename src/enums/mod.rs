use std::fmt::Display;

use postgres_types::{FromSql, ToSql};
use serde::Deserialize;
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
