use serde::Deserialize;
use postgres_types::{ FromSql, ToSql };
#[derive(Deserialize, Debug, ToSql, FromSql)]
#[serde(rename_all = "snake_case")]
#[postgres(name = "ImageType")]
pub enum ImageType {
    #[postgres(name = "images")]
    Images,
    #[postgres(name = "map_images")]
    MapImages,
}
