#[derive(Debug, Clone)]
pub struct FieldConfiguration {
    pub body_format: Option<String>,
    pub include_version: bool,
    pub include_labels: bool,
    pub include_properties: bool,
    pub include_operations: bool,
    pub custom_includes: Vec<String>,
    pub include_all: bool,
}

impl Default for FieldConfiguration {
    fn default() -> Self {
        Self {
            body_format: Some("storage".to_string()),
            include_version: true,
            include_labels: false,
            include_properties: false,
            include_operations: false,
            custom_includes: vec![],
            include_all: false,
        }
    }
}

impl FieldConfiguration {
    pub fn from_env() -> Self {
        let custom_includes = std::env::var("CONFLUENCE_CUSTOM_INCLUDES")
            .ok()
            .filter(|s| !s.is_empty())
            .map(|s| {
                s.split(',')
                    .filter(|p| !p.is_empty())
                    .map(|p| p.trim().to_string())
                    .collect()
            })
            .unwrap_or_default();

        Self {
            custom_includes,
            ..Default::default()
        }
    }

    pub fn all_fields() -> Self {
        Self {
            body_format: Some("storage".to_string()),
            include_version: true,
            include_labels: true,
            include_properties: true,
            include_operations: true,
            custom_includes: vec![],
            include_all: true,
        }
    }

    pub fn with_additional_includes(mut self, additional: Vec<String>) -> Self {
        for param in additional {
            if !self.custom_includes.contains(&param) {
                self.custom_includes.push(param);
            }
        }
        self
    }

    pub fn to_query_params(&self) -> Vec<(String, String)> {
        let mut params = Vec::new();

        if let Some(ref format) = self.body_format {
            params.push(("body-format".to_string(), format.clone()));
        }

        if self.include_version {
            params.push(("include-version".to_string(), "true".to_string()));
        }
        if self.include_labels || self.include_all {
            params.push(("include-labels".to_string(), "true".to_string()));
        }
        if self.include_properties || self.include_all {
            params.push(("include-properties".to_string(), "true".to_string()));
        }
        if self.include_operations || self.include_all {
            params.push(("include-operations".to_string(), "true".to_string()));
        }

        for param in &self.custom_includes {
            params.push((format!("include-{}", param), "true".to_string()));
        }

        params
    }
}

pub fn apply_v2_filtering(
    include_all_fields: Option<bool>,
    additional_includes: Option<Vec<String>>,
) -> Vec<(String, String)> {
    if include_all_fields.unwrap_or(false) {
        return FieldConfiguration::all_fields().to_query_params();
    }

    let mut config = FieldConfiguration::from_env();

    if let Some(additional) = additional_includes {
        config = config.with_additional_includes(additional);
    }

    config.to_query_params()
}

pub fn apply_expand_filtering(
    url: &str,
    include_all_fields: Option<bool>,
    additional_expand: Option<Vec<String>>,
) -> (String, Option<String>) {
    let expand_params = if include_all_fields.unwrap_or(false) {
        vec!["body.storage", "version", "space", "history", "metadata"]
    } else {
        vec!["body.storage", "version"]
    };

    let mut expand: Vec<String> = expand_params.iter().map(|s| s.to_string()).collect();

    if let Some(additional) = additional_expand {
        for param in additional {
            if !expand.contains(&param) {
                expand.push(param);
            }
        }
    }

    (url.to_string(), Some(expand.join(",")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_params() {
        let config = FieldConfiguration::default();
        assert_eq!(config.body_format, Some("storage".to_string()));
        assert!(config.include_version);
        assert!(!config.include_labels);
        assert!(config.custom_includes.is_empty());
    }

    #[test]
    fn test_all_fields() {
        let config = FieldConfiguration::all_fields();
        assert!(config.include_version);
        assert!(config.include_labels);
        assert!(config.include_properties);
        assert!(config.include_operations);
        assert!(config.include_all);
    }

    #[test]
    fn test_query_params_default() {
        let config = FieldConfiguration::default();
        let params = config.to_query_params();

        assert_eq!(params.len(), 2);
        assert!(params.contains(&("body-format".to_string(), "storage".to_string())));
        assert!(params.contains(&("include-version".to_string(), "true".to_string())));
    }

    #[test]
    fn test_query_params_all_fields() {
        let config = FieldConfiguration::all_fields();
        let params = config.to_query_params();

        assert_eq!(params.len(), 5);
        assert!(params.contains(&("include-labels".to_string(), "true".to_string())));
        assert!(params.contains(&("include-properties".to_string(), "true".to_string())));
        assert!(params.contains(&("include-operations".to_string(), "true".to_string())));
    }

    #[test]
    fn test_with_additional_includes() {
        let config = FieldConfiguration::default()
            .with_additional_includes(vec!["ancestors".to_string(), "children".to_string()]);

        assert_eq!(config.custom_includes.len(), 2);
        assert!(config.custom_includes.contains(&"ancestors".to_string()));
    }

    #[test]
    fn test_custom_includes_query_params() {
        let mut config = FieldConfiguration::default();
        config.custom_includes = vec!["ancestors".to_string(), "history".to_string()];
        let params = config.to_query_params();

        assert_eq!(params.len(), 4);
        assert!(params.contains(&("include-ancestors".to_string(), "true".to_string())));
        assert!(params.contains(&("include-history".to_string(), "true".to_string())));
    }

    #[test]
    fn test_expand_filtering_default() {
        let (url, expand) = apply_expand_filtering(
            "https://test.atlassian.net/wiki/rest/api/search",
            None,
            None,
        );

        assert_eq!(url, "https://test.atlassian.net/wiki/rest/api/search");
        assert_eq!(expand, Some("body.storage,version".to_string()));
    }

    #[test]
    fn test_expand_filtering_all_fields() {
        let (_, expand) = apply_expand_filtering(
            "https://test.atlassian.net/wiki/rest/api/search",
            Some(true),
            None,
        );

        assert_eq!(
            expand,
            Some("body.storage,version,space,history,metadata".to_string())
        );
    }

    #[test]
    fn test_expand_filtering_with_additional() {
        let additional = vec!["ancestors".to_string(), "children".to_string()];
        let (_, expand) = apply_expand_filtering(
            "https://test.atlassian.net/wiki/rest/api/search",
            None,
            Some(additional),
        );

        let expand_str = expand.unwrap();
        assert!(expand_str.contains("ancestors"));
        assert!(expand_str.contains("children"));
    }
}
