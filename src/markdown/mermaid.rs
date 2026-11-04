use mermaid_rs_renderer::{LayoutConfig, Theme};
use serde_json::{Value, json};

pub fn settings(init: Option<&Value>, dark: bool) -> Result<(Theme, LayoutConfig), String> {
    let mut theme = if dark { Theme::dark() } else { Theme::modern() };
    let Some(init) = init else {
        return Ok((theme, LayoutConfig::default()));
    };
    let init = init.as_object().ok_or("Mermaid init must be an object")?;
    if let Some(name) = init.get("theme") {
        let name = name.as_str().ok_or("Mermaid theme must be a string")?;
        theme = if name == "base" {
            Theme::mermaid_default()
        } else {
            Theme::from_name(name).ok_or_else(|| format!("Unsupported Mermaid theme: {name}"))?
        };
    }
    let mut theme = serde_json::to_value(theme).map_err(|e| e.to_string())?;
    let mut layout = serde_json::to_value(LayoutConfig::default()).map_err(|e| e.to_string())?;
    for (key, value) in init {
        match key.as_str() {
            "theme" => {}
            "themeVariables" => {
                for (key, value) in value
                    .as_object()
                    .ok_or("themeVariables must be an object")?
                {
                    let key = match key.as_str() {
                        "clusterBkg" => "cluster_background".into(),
                        "actorBkg" => "sequence_actor_fill".into(),
                        "actorBorder" => "sequence_actor_border".into(),
                        "actorLineColor" => "sequence_actor_line".into(),
                        "noteBkgColor" => "sequence_note_fill".into(),
                        "noteBorderColor" => "sequence_note_border".into(),
                        "activationBkgColor" => "sequence_activation_fill".into(),
                        "activationBorderColor" => "sequence_activation_border".into(),
                        _ => snake(key),
                    };
                    if let Some(index) = indexed(&key, "pie", 1, 12) {
                        set(&mut theme["pie_colors"][index], "color", value)?;
                    } else if let Some(index) = indexed(&key, "git", 0, 8) {
                        set(&mut theme["git_colors"][index], "color", value)?;
                    } else if let Some(index) = indexed(&key, "git_inv", 0, 8) {
                        set(&mut theme["git_inv_colors"][index], "color", value)?;
                    } else if let Some(index) = indexed(&key, "git_branch_label", 0, 8) {
                        set(&mut theme["git_branch_label_colors"][index], "color", value)?;
                    } else {
                        let target = theme
                            .get_mut(&key)
                            .ok_or_else(|| format!("Unsupported Mermaid theme variable: {key}"))?;
                        set(target, &key, value)?;
                    }
                }
            }
            "fontFamily" | "fontSize" => {
                let key = snake(key);
                set(&mut theme[&key], &key, value)?;
            }
            "flowchart" => {
                for (key, value) in value.as_object().ok_or("flowchart must be an object")? {
                    let field = snake(key);
                    if matches!(
                        field.as_str(),
                        "node_spacing"
                            | "rank_spacing"
                            | "node_padding_x"
                            | "node_padding_y"
                            | "label_line_height"
                    ) {
                        set(&mut layout[&field], &field, value)?;
                    } else {
                        let object = json!({key: value});
                        merge(&mut layout["flowchart"], &object)?;
                    }
                }
            }
            "gitGraph" | "mindmap" | "pie" | "timeline" | "requirement" | "c4" | "treemap" => {
                let field = if key == "gitGraph" { "gitgraph" } else { key };
                merge(&mut layout[field], value)?;
            }
            _ => return Err(format!("Unsupported Mermaid init setting: {key}")),
        }
    }
    let theme: Theme =
        serde_json::from_value(theme).map_err(|e| format!("Invalid Mermaid theme: {e}"))?;
    let layout: LayoutConfig =
        serde_json::from_value(layout).map_err(|e| format!("Invalid Mermaid layout: {e}"))?;
    Ok((theme, layout))
}

fn snake(key: &str) -> String {
    let mut result = String::new();
    for ch in key.chars() {
        if ch.is_ascii_uppercase() {
            result.push('_');
            result.push(ch.to_ascii_lowercase());
        } else {
            result.push(ch);
        }
    }
    result
}

fn indexed(key: &str, prefix: &str, first: usize, count: usize) -> Option<usize> {
    key.strip_prefix(prefix)?
        .parse::<usize>()
        .ok()?
        .checked_sub(first)
        .filter(|ix| *ix < count)
}

fn merge(target: &mut Value, source: &Value) -> Result<(), String> {
    for (key, value) in source
        .as_object()
        .ok_or("Mermaid settings must be objects")?
    {
        let field = if target.get(key).is_some() {
            key.clone()
        } else {
            snake(key)
        };
        let target = target
            .get_mut(&field)
            .ok_or_else(|| format!("Unsupported Mermaid setting: {key}"))?;
        if target.is_object() {
            merge(target, value)?;
        } else {
            set(target, &field, value)?;
        }
    }
    Ok(())
}

fn set(target: &mut Value, field: &str, value: &Value) -> Result<(), String> {
    let value = if field.ends_with("font_size") || field.ends_with("text_size") {
        let size = match value {
            Value::String(s) => s.strip_suffix("px").unwrap_or(s).trim().parse::<f64>().ok(),
            _ => value.as_f64(),
        }
        .ok_or("Invalid Mermaid font size")?;
        if !size.is_finite() || !(1.0..=96.0).contains(&size) {
            return Err("Mermaid font sizes must be between 1 and 96".into());
        }
        json!(size)
    } else {
        value.clone()
    };
    if let Some(number) = value.as_f64() {
        let limit = if field.contains("passes") { 32. } else { 2048. };
        if !number.is_finite()
            || number.abs() > limit
            || (field.contains("line_height") && number <= 0.)
        {
            return Err(format!("Mermaid setting {field} is outside safe limits"));
        }
    }
    if let Some(text) = value.as_str() {
        if text
            .chars()
            .any(|ch| ch.is_control() || "<>\"&".contains(ch))
            || text.to_ascii_lowercase().contains("url(")
        {
            return Err(format!("Unsafe Mermaid setting: {field}"));
        }
        if field != "font_family"
            && target.is_string()
            && (field.contains("color")
                || field.contains("fill")
                || field.contains("border")
                || field.contains("background"))
        {
            text.parse::<svgtypes::Color>()
                .map_err(|_| format!("Invalid Mermaid color: {field}"))?;
        }
    }
    if !(target.is_number() && value.is_number()
        || target.is_string() && value.is_string()
        || target.is_boolean() && value.is_boolean()
        || target.is_null())
    {
        return Err(format!("Invalid type for Mermaid setting: {field}"));
    }
    *target = value;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applies_theme_variables_and_layout_instead_of_ignoring_init() {
        let (theme, layout) = settings(Some(&json!({"theme":"base", "themeVariables":{"primaryColor":"#ff0000", "fontFamily":"Arial", "fontSize":"20px", "actorBkg":"#00ff00", "pie1":"#0000ff", "gitInv0":"#123456"}, "flowchart":{"nodeSpacing":120,"rankSpacing":90}, "gitGraph":{"showBranches":false}, "timeline":{"direction":"TB"}})), true).unwrap();
        assert_eq!(theme.primary_color, "#ff0000");
        assert_eq!(theme.font_size, 20.);
        assert_eq!(theme.sequence_actor_fill, "#00ff00");
        assert_eq!(theme.pie_colors[0], "#0000ff");
        assert_eq!(theme.git_inv_colors[0], "#123456");
        assert_eq!(layout.node_spacing, 120.);
        assert_eq!(layout.rank_spacing, 90.);
        assert!(!layout.gitgraph.show_branches);
        assert_eq!(layout.timeline.direction, "TB");
    }

    #[test]
    fn rejects_unknown_unsafe_or_excessive_options() {
        for init in [
            json!({"unknown":true}),
            json!({"flowchart":{"unknown":1}}),
            json!({"themeVariables":{"unknown":"red"}}),
            json!({"themeVariables":{"primaryColor":"red\"/><image href=\"file:///x"}}),
            json!({"themeVariables":{"primaryColor":"url(file:///x)"}}),
            json!({"fontSize":0}),
            json!({"fontSize":1000}),
            json!({"flowchart":{"nodeSpacing":100000}}),
            json!({"flowchart":{"orderPasses":1000}}),
        ] {
            assert!(settings(Some(&init), false).is_err(), "{init}");
        }
    }
}
