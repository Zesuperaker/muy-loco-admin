use loco_rs::prelude::*;
use serde::Deserialize;

pub use super::_entities::articles::{ActiveModel, Entity, Model};
pub use super::_entities::sea_orm_active_enums::ArticleStatus;
pub type Articles = Entity;

#[derive(Debug, Validate, Deserialize)]
#[validate(crate = "loco_rs::validator")]
pub struct Validator {
    #[validate(length(min = 1, message = "title must not be empty"))]
    pub title: String,
    #[validate(range(min = 0.0, max = 5.0, message = "rating must be between 0 and 5"))]
    pub rating: Option<f64>,
}

impl Validatable for ActiveModel {
    fn validator(&self) -> Box<dyn Validate> {
        Box::new(Validator {
            title: self.title.try_as_ref().cloned().unwrap_or_default(),
            rating: self.rating.try_as_ref().copied().flatten(),
        })
    }
}

#[async_trait]
impl ActiveModelBehavior for ActiveModel {
    async fn before_save<C>(self, _db: &C, insert: bool) -> std::result::Result<Self, DbErr>
    where
        C: ConnectionTrait,
    {
        if insert || self.title.is_set() || self.rating.is_set() {
            self.validate()?;
        }
        if !insert && self.updated_at.is_unchanged() {
            let mut this = self;
            this.updated_at = ActiveValue::Set(chrono::Utc::now().into());
            Ok(this)
        } else {
            Ok(self)
        }
    }
}
