use sea_orm::entity::prelude::*;

pub use super::_entities::article_tags::{ActiveModel, Entity, Model};

impl ActiveModelBehavior for ActiveModel {}
