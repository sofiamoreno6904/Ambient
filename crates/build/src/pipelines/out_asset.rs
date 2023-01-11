use std::{
    collections::{HashMap, HashSet}, io::Cursor, sync::Arc
};

use elements_std::asset_url::{AbsAssetUrl, AssetType};

use crate::pipelines::context::AssetCrateId;

#[derive(Debug)]
pub enum OutAssetContent {
    Content(AbsAssetUrl),
    Collection(Vec<AssetCrateId>),
}
impl OutAssetContent {
    pub fn is_collection(&self) -> bool {
        matches!(self, OutAssetContent::Collection(..))
    }
}

#[derive(Debug, Clone)]
pub enum OutAssetPreview {
    None,
    FromModel { url: AbsAssetUrl },
    Image { image: Arc<image::RgbaImage> },
}

#[derive(Debug)]
pub struct OutAsset {
    pub asset_crate_id: AssetCrateId,
    /// Which asset in the asset crate is this
    pub sub_asset: Option<String>,
    pub type_: AssetType,
    /// If this asset is not displayed in search results
    pub hidden: bool,
    pub name: String,
    pub tags: Vec<String>,
    /// Each entry in the vec is a category level, i.e.:
    /// self.categories[0].insert("Vehicles");
    /// self.categories[1].insert("Vehicles > Cars");
    pub categories: [HashSet<String>; 3],
    pub preview: OutAssetPreview,
    pub content: OutAssetContent,
    pub source: Option<String>,
}
