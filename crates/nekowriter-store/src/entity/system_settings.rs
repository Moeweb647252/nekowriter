use nekowriter_core::{
    model::{ApiBaseUrl, ApiKey, ModelProfile},
    time::Timestamp,
};
use sea_orm::entity::prelude::*;

/// The primary key of the only row.
pub(crate) const ROW_ID: i32 = 1;

/// The model service and the default model, shared by all projects.
#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "system_settings")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: i32,
    pub api_base_url: Option<ApiBaseUrl>,
    /// Read only to authenticate requests (§4 invariant 17).
    pub api_key: Option<ApiKey>,
    pub default_model: Option<ModelProfile>,
    pub revision: i64,
    pub updated_at: Timestamp,
}

impl ActiveModelBehavior for ActiveModel {}
