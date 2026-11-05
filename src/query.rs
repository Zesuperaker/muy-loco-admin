//! List query parameters.

use std::collections::HashMap;

use crate::{
    error::{AdminError, AdminResult},
    meta::SortDirection,
};

/// Largest accepted page size.
pub const MAX_PER_PAGE: u64 = 500;

/// Filter value matching `NULL`.
pub const NULL_FILTER: &str = "__null__";
/// Filter value matching anything but `NULL`.
pub const NOT_NULL_FILTER: &str = "__notnull__";

const FILTER_PREFIX: &str = "filter.";

/// Parsed `?page=&per_page=&sort=&order=&q=&filter.<field>=` parameters.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ListParams {
    pub page: u64,
    pub per_page: Option<u64>,
    pub sort: Option<String>,
    pub order: Option<SortDirection>,
    pub q: Option<String>,
    pub filters: Vec<(String, String)>,
}

fn parse_u64(name: &str, raw: &str) -> AdminResult<u64> {
    raw.trim()
        .parse()
        .map_err(|_| AdminError::bad_request(format!("`{name}` must be a positive integer")))
}

impl ListParams {
    pub(crate) fn from_query(query: &HashMap<String, String>) -> AdminResult<Self> {
        let mut params = Self {
            page: 1,
            ..Self::default()
        };
        for (key, raw) in query {
            match key.as_str() {
                "page" => params.page = parse_u64("page", raw)?.max(1),
                "per_page" => {
                    params.per_page = Some(parse_u64("per_page", raw)?.clamp(1, MAX_PER_PAGE));
                }
                "sort" if !raw.is_empty() => params.sort = Some(raw.clone()),
                "order" => {
                    params.order = match raw.to_ascii_lowercase().as_str() {
                        "asc" => Some(SortDirection::Asc),
                        "desc" => Some(SortDirection::Desc),
                        "" => None,
                        _ => return Err(AdminError::bad_request("`order` must be asc or desc")),
                    };
                }
                "q" => {
                    let q = raw.trim();
                    if !q.is_empty() {
                        params.q = Some(q.to_string());
                    }
                }
                other => {
                    if let Some(field) = other.strip_prefix(FILTER_PREFIX)
                        && !raw.is_empty()
                    {
                        params.filters.push((field.to_string(), raw.clone()));
                    }
                }
            }
        }
        params.filters.sort();
        Ok(params)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    #[test]
    fn defaults() {
        let p = ListParams::from_query(&HashMap::new()).unwrap();
        assert_eq!(p.page, 1);
        assert_eq!(p.per_page, None);
        assert!(p.filters.is_empty());
    }

    #[test]
    fn parses_everything() {
        let p = ListParams::from_query(&q(&[
            ("page", "3"),
            ("per_page", "10000"),
            ("sort", "title"),
            ("order", "ASC"),
            ("q", "  rust "),
            ("filter.published", "true"),
            ("filter.user_id", "4"),
            ("filter.empty", ""),
            ("unrelated", "x"),
        ]))
        .unwrap();
        assert_eq!(p.page, 3);
        assert_eq!(p.per_page, Some(MAX_PER_PAGE));
        assert_eq!(p.sort.as_deref(), Some("title"));
        assert_eq!(p.order, Some(SortDirection::Asc));
        assert_eq!(p.q.as_deref(), Some("rust"));
        assert_eq!(
            p.filters,
            vec![
                ("published".to_string(), "true".to_string()),
                ("user_id".to_string(), "4".to_string())
            ]
        );
    }

    #[test]
    fn edge_values() {
        let p = ListParams::from_query(&q(&[
            ("page", "0"),
            ("q", "  "),
            ("order", ""),
            ("sort", ""),
        ]))
        .unwrap();
        assert_eq!(p.page, 1);
        assert_eq!(p.q, None);
        assert_eq!(p.order, None);
        assert_eq!(p.sort, None);
        let p = ListParams::from_query(&q(&[("order", "desc"), ("per_page", "0")])).unwrap();
        assert_eq!(p.order, Some(SortDirection::Desc));
        assert_eq!(p.per_page, Some(1));
    }

    #[test]
    fn rejects_bad_values() {
        assert!(ListParams::from_query(&q(&[("page", "x")])).is_err());
        assert!(ListParams::from_query(&q(&[("per_page", "-1")])).is_err());
        assert!(ListParams::from_query(&q(&[("order", "sideways")])).is_err());
    }
}
