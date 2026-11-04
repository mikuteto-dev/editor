use std::sync::Once;

use gpui_kit::component::highlighter::{GrammarConfig, LanguageRegistry};

pub fn init() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        LanguageRegistry::singleton().register(
            "vue",
            &GrammarConfig::new(
                "vue",
                tree_sitter_vue_next::LANGUAGE.into(),
                ["javascript", "typescript", "tsx", "css"]
                    .into_iter()
                    .map(Into::into)
                    .collect(),
                &format!(
                    "{}\n{}",
                    include_str!(
                        "../vendor/gpui-component/src/highlighter/languages/html/highlights.scm"
                    ),
                    include_str!("syntax/vue/highlights.scm"),
                ),
                include_str!("syntax/vue/injections.scm"),
                "",
            ),
        );
    });
}

#[cfg(test)]
mod tests {
    use gpui_kit::component::{
        highlighter::{HighlightTheme, SyntaxHighlighter},
        input::Rope,
    };

    fn check(language: &str, code: &str, tokens: &[(&str, &str)]) {
        super::init();
        let theme = HighlightTheme::default_dark();
        let mut highlighter = SyntaxHighlighter::new(language);
        assert_eq!(highlighter.language().as_ref(), language);
        highlighter.update(None, &Rope::from(code), None);
        let styles = highlighter.styles(&(0..code.len()), &*theme);
        for &(token, capture) in tokens {
            let start = code.find(token).unwrap();
            let expected = theme.style(capture).unwrap();
            assert!(
                styles.iter().any(|(range, style)| {
                    range.start <= start
                        && range.end >= start + token.len()
                        && style.color == expected.color
                }),
                "{language}: {token:?} should be {capture}; got {styles:?}",
            );
        }
    }

    #[test]
    fn enabled_queries_parse() {
        use gpui_kit::component::highlighter::Language;

        for language in Language::all().filter(|language| *language != Language::Plain) {
            let mut highlighter = SyntaxHighlighter::new(language.name());
            highlighter.update(None, &Rope::from("\n"), None);
            assert!(highlighter.tree().is_some(), "{}", language.name());
        }
    }

    #[test]
    fn vue() {
        check(
            "vue",
            "<script setup lang=\"ts\">\nconst count: number = 42;\n</script>\n<template><button @click=\"count += 3\">{{ count + 7 }}</button></template>\n<style scoped>.button { color: #f00; }</style>",
            &[
                ("script", "tag"),
                ("const", "keyword"),
                ("number", "type"),
                ("42", "number"),
                ("click", "attribute"),
                ("3", "number"),
                ("7", "number"),
                ("#f00", "string.special"),
            ],
        );
        check(
            "vue",
            "<script>const message = 'hello';</script>",
            &[("const", "keyword"), ("'hello'", "string")],
        );
        check(
            "vue",
            "<script setup lang=ts>const count: number = 42;</script>",
            &[("number", "type"), ("42", "number")],
        );
        check(
            "vue",
            "<style lang=\"css\">p { color: #f00; }</style>",
            &[("#f00", "string.special")],
        );
        check(
            "vue",
            "<template>{{ '日本語' }}</template>",
            &[("'日本語'", "string")],
        );
        check(
            "vue",
            "<script lang=\"unknown\">some unfinished code",
            &[("script", "tag")],
        );
        check(
            "markdown",
            "```vue\n<template>{{ 42 }}</template>\n```",
            &[("template", "tag")],
        );
    }

    #[test]
    fn web_components() {
        check(
            "svelte",
            "<script lang=\"ts\">const count: number = 42;</script>\n<button>{count + 7}</button>\n<style>button { color: #f00; }</style>",
            &[
                ("const", "keyword"),
                ("number", "type"),
                ("7", "number"),
                ("#f00", "string.special"),
            ],
        );
        check(
            "astro",
            "---\nconst title = 'hello';\n---\n<h1>{title + 7}</h1>\n<style>h1 { color: #f00; }</style>",
            &[
                ("const", "keyword"),
                ("'hello'", "string"),
                ("7", "number"),
                ("#f00", "string.special"),
            ],
        );
    }

    #[test]
    fn existing_languages() {
        check(
            "csharp",
            "class Program { string name = \"hello\"; }",
            &[("class", "keyword"), ("\"hello\"", "string")],
        );
        check(
            "swift",
            "let name = \"hello\"",
            &[("let", "keyword"), ("hello", "string")],
        );
        check("cmake", "set(NAME \"hello\")", &[("\"hello\"", "string")]);
        check(
            "tsx",
            "const view = <button title=\"hello\">{42}</button>;",
            &[
                ("const", "keyword"),
                ("button", "tag"),
                ("\"hello\"", "string"),
                ("42", "number"),
            ],
        );
        check(
            "javascript",
            "const view = <button title=\"hello\">{42}</button>;",
            &[("button", "tag"), ("title", "attribute")],
        );
    }
}
