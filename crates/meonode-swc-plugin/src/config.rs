//! Plugin configuration — the `{}` second element of the `swcPlugins` tuple:
//! `[['@meonode/compiler', { factoryModules: ['@meonode/mui'] }]]`.
//!
//! See the v0.2 design doc's "Factory recognition beyond @meonode/ui" section.
//! Packages that wrap `Node()` in their own factory helper (e.g.
//! `@meonode/mui`'s `createMuiNode`) are invisible to per-file detection,
//! since detection only traces `@meonode/ui` imports and same-file
//! `createNode`/`createChildrenFirstNode` bindings. `factoryModules` lets
//! users opt a wrapping package's capitalized named exports in as
//! props-at-arg-0 factories, same as a plain `@meonode/ui` HTML factory.

use serde::Deserialize;

/// Parsed plugin configuration. Every field must have a safe, current-
/// behavior default so a missing or malformed config degrades to "compile
/// nothing extra" rather than failing the build — see [`CompileConfig::from_json`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CompileConfig {
    /// Module specifiers (e.g. `"@meonode/mui"`) whose capitalized named
    /// imports should be treated as props-at-arg-0 factories. Lowercase
    /// named imports (helper functions, e.g. `createMuiNode`,
    /// `isProbablyMuiTheme`) are always ignored, regardless of this list.
    /// Empty by default: no extra modules are recognized beyond
    /// `@meonode/ui`/`@meonode/ui/client`, matching pre-v0.2 behavior.
    pub factory_modules: Vec<String>,
}

impl CompileConfig {
    /// Parses the plugin's raw JSON config string (see
    /// `TransformPluginProgramMetadata::get_transform_plugin_config`, which
    /// itself returns `None` when the host provides no config — including
    /// every non-wasm32 target, e.g. `cargo test`). Falls back to
    /// [`CompileConfig::default`] (empty `factory_modules`) on `None` *or* a
    /// malformed/unparseable string: a bad plugin config must never fail the
    /// build, only silently forgo the extra recognition it would have
    /// enabled.
    pub fn from_json(json: Option<&str>) -> Self {
        json.and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn none_yields_default_empty_config() {
        assert_eq!(CompileConfig::from_json(None), CompileConfig::default());
        assert!(CompileConfig::from_json(None).factory_modules.is_empty());
    }

    #[test]
    fn parses_factory_modules_list() {
        let config = CompileConfig::from_json(Some(r#"{"factoryModules":["@meonode/mui"]}"#));
        assert_eq!(config.factory_modules, vec!["@meonode/mui".to_string()]);
    }

    #[test]
    fn parses_multiple_factory_modules() {
        let config = CompileConfig::from_json(Some(
            r#"{"factoryModules":["@meonode/mui","@acme/widgets"]}"#,
        ));
        assert_eq!(
            config.factory_modules,
            vec!["@meonode/mui".to_string(), "@acme/widgets".to_string()]
        );
    }

    #[test]
    fn empty_json_object_yields_default() {
        let config = CompileConfig::from_json(Some("{}"));
        assert_eq!(config, CompileConfig::default());
    }

    #[test]
    fn malformed_json_degrades_to_default_rather_than_panicking() {
        let config = CompileConfig::from_json(Some("not json at all"));
        assert_eq!(config, CompileConfig::default());
    }

    #[test]
    fn wrong_shaped_json_degrades_to_default() {
        // A JSON array instead of an object: still must not panic, and must
        // still produce a safe, empty config.
        let config = CompileConfig::from_json(Some(r#"["@meonode/mui"]"#));
        assert_eq!(config, CompileConfig::default());
    }
}
