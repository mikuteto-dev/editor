use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use gpui_kit::{
    base::input::{Diagnostic, DiagnosticSeverity},
    component::{
        ActiveTheme, WindowExt,
        input::{HoverProvider, Position, RangeDecoration, RangeDecorationStyle, Rope, RopeExt},
    },
    *,
};
use lsp_types::{Hover, HoverContents, MarkupContent, MarkupKind};

use super::Workspace;
use crate::{
    commands::{DependencyGraph, DependencyLicenses},
    dep_graph::DepGraph,
    licenses::{self, Line, Lines, Report, Verdict, find, verdict},
    tab::{Body, PreviewKind, Tab},
};

pub struct LicenseHover(pub Lines);

impl HoverProvider for LicenseHover {
    fn hover(&self, text: &Rope, offset: usize, _: &mut Window, _: &mut App) -> Task<anyhow::Result<Option<Hover>>> {
        let line = text.offset_to_position(offset).line;
        let hover = self.0.borrow().get(&line).map(|l| Hover {
            contents: HoverContents::Markup(MarkupContent { kind: MarkupKind::Markdown, value: l.markdown() }),
            range: None,
        });
        Task::ready(Ok(hover))
    }
}

impl Workspace {
    /// One report per manifest serves every open file under it.
    pub(super) fn check_licenses(&mut self, ix: usize, cx: &mut Context<Self>) {
        let (Some(root), Some(tab)) = (self.root.clone(), self.tabs.get(ix)) else { return };
        if tab.text().is_none_or(|t| t.readonly) {
            return;
        }
        let Some(manifest) = licenses::manifest_for(&tab.path, &root) else { return };
        match self.reports.get(&manifest) {
            Some(Some(report)) => {
                let report = report.clone();
                self.apply_licenses(ix, &manifest, &report, cx);
            }
            // Already being read; it applies to this tab when it lands.
            Some(None) => {}
            None => {
                self.reports.insert(manifest.clone(), None);
                cx.spawn(async move |this, cx| {
                    let report = cx
                        .background_spawn({
                            let manifest = manifest.clone();
                            async move { licenses::report(&manifest).unwrap_or_default() }
                        })
                        .await;
                    this.update(cx, |this, cx| {
                        this.reports.insert(manifest.clone(), Some(Arc::new(report)));
                        let Some(root) = this.root.clone() else { return };
                        for ix in 0..this.tabs.len() {
                            if licenses::manifest_for(&this.tabs[ix].path, &root).as_ref() == Some(&manifest) {
                                this.check_licenses(ix, cx);
                            }
                        }
                    })
                    .ok();
                })
                .detach();
            }
        }
    }

    fn apply_licenses(&mut self, ix: usize, manifest: &Path, report: &Report, cx: &mut Context<Self>) {
        let tab = &self.tabs[ix];
        let Body::Text(text) = &tab.body else { return };
        let source = text.editor.read(cx).text().to_string();
        let (mentions, own_line) = licenses::mentions(&tab.path, manifest, &source);
        let own = report.own.as_deref();
        let mut lines = HashMap::new();
        let mut diagnostics = Vec::new();
        let mut mentioned = Vec::new();
        for m in &mentions {
            let Some(dep) = find(&report.deps, &m.name) else { continue };
            mentioned.push(dep as *const _);
            let verdict = if dep.shipped { verdict(own, dep.license.as_deref()) } else { Verdict::Ok };
            let version = if dep.version.is_empty() { String::new() } else { format!(" {}", dep.version) };
            let usage = if dep.shipped { "" } else { " · dev only" };
            let license = dep.license.as_deref().unwrap_or("no license");
            let summary = format!("{}{version} · {license}{usage}", dep.name);
            let label = format!("{license}{usage}");
            let range = Position::new(m.line, m.cols.start)..Position::new(m.line, m.cols.end);
            let line = Line { summary, label, verdict };
            if let Some(severity) = severity(&line.verdict) {
                diagnostics.push(Diagnostic::new(range, line.markdown()).with_severity(severity));
            }
            lines.insert(m.line, line);
        }
        // Conflicts deep in the tree have no line of their own; the package's license line carries them.
        let deep: Vec<String> = report
            .deps
            .iter()
            .filter(|d| d.shipped && !mentioned.contains(&(*d as *const _)))
            .filter(|d| matches!(verdict(own, d.license.as_deref()), Verdict::Conflict(_)))
            .map(|d| format!("{} {} ({})", d.name, d.version, d.license.as_deref().unwrap_or("no license")))
            .collect();
        if let Some(m) = own_line {
            let label = own.unwrap_or("no license").to_string();
            let summary = format!("This package: {label}");
            let verdict = match deep.is_empty() {
                true => Verdict::Ok,
                false => Verdict::Conflict(format!("Indirect dependencies conflict with it:\n\n{}", deep.join("\n\n"))),
            };
            let line = Line { summary, label, verdict };
            if let Some(severity) = severity(&line.verdict) {
                let range = Position::new(m.line, m.cols.start)..Position::new(m.line, m.cols.end);
                diagnostics.push(Diagnostic::new(range, line.markdown()).with_severity(severity));
            }
            lines.insert(m.line, line);
        }
        let theme = cx.theme();
        let rope = text.editor.read(cx).text().clone();
        let labels = lines
            .iter()
            .map(|(&line, l)| {
                let color = match l.verdict {
                    Verdict::Ok => theme.muted_foreground.opacity(0.7),
                    Verdict::Caution(_) | Verdict::Unknown(_) => theme.warning,
                    Verdict::Conflict(_) => theme.danger,
                };
                let start = rope.position_to_offset(&Position::new(line, 0));
                let end = rope.position_to_offset(&Position::new(line + 1, 0));
                RangeDecoration::new(start..end)
                    .with_style(RangeDecorationStyle::Label)
                    .with_color(color)
                    .with_label(l.label.clone())
            })
            .collect();
        text.licenses.replace(lines);
        let editor = text.editor.clone();
        match &text.license_labels {
            Some(set) => set.set(labels, cx),
            None => {
                let set = editor.update(cx, |editor, cx| editor.create_range_decorations_collection(labels, cx));
                if let Body::Text(text) = &mut self.tabs[ix].body {
                    text.license_labels = Some(set);
                }
            }
        }
        editor.update(cx, |editor, cx| {
            let rope = editor.text().clone();
            if let Some(set) = editor.diagnostics_mut() {
                set.reset(&rope);
                set.extend(diagnostics);
            }
            cx.notify();
        });
        cx.notify();
    }

    fn pick_manifest(&self) -> Option<PathBuf> {
        let root = self.root.clone()?;
        let active = self.tabs.get(self.active).and_then(|t| licenses::manifest_for(&t.path, &root));
        active.or_else(|| {
            let mut entries: Vec<PathBuf> = std::fs::read_dir(&root).ok()?.flatten().map(|e| e.path()).collect();
            entries.sort();
            entries.into_iter().find(|p| licenses::is_manifest(p))
        })
    }

    pub(super) fn show_graph(&mut self, _: &DependencyGraph, window: &mut Window, cx: &mut Context<Self>) {
        let Some(manifest) = self.pick_manifest() else {
            return window.push_notification("No package manifest here", cx);
        };
        let cached = self.reports.get(&manifest).cloned().flatten();
        cx.spawn_in(window, async move |this, cx| {
            let report = match cached {
                Some(report) => Some(report),
                None => {
                    let manifest = manifest.clone();
                    cx.background_spawn(async move { licenses::report(&manifest).map(Arc::new) }).await
                }
            };
            this.update_in(cx, |this, window, cx| {
                let Some(report) = report else {
                    return window.push_notification(format!("{}: cannot read dependencies", manifest.display()), cx);
                };
                let key = PathBuf::from(format!("graph:{}", manifest.display()));
                this.tabs.retain(|t| t.path != key);
                let view = cx.new(|cx| DepGraph::new(report, window, cx));
                this.insert(Tab { path: key, name: "Dependencies".into(), body: Body::Graph(view), pinned: true }, window, cx);
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn show_licenses(&mut self, _: &DependencyLicenses, window: &mut Window, cx: &mut Context<Self>) {
        let Some(manifest) = self.pick_manifest() else {
            return window.push_notification("No package manifest here", cx);
        };
        cx.spawn_in(window, async move |this, cx| {
            let report = cx
                .background_spawn({
                    let manifest = manifest.clone();
                    async move { licenses::report(&manifest) }
                })
                .await;
            this.update_in(cx, |this, window, cx| {
                let Some(report) = report else {
                    return window.push_notification(format!("{}: cannot read dependencies", manifest.display()), cx);
                };
                let key = PathBuf::from(format!("licenses:{}", manifest.display()));
                this.tabs.retain(|t| t.path != key);
                let page = page(&report);
                let mut body = this.text(&key, page.clone(), "markdown", true, window, cx);
                if let Body::Text(text) = &mut body {
                    text.preview = Some(PreviewKind::Markdown.render(&page));
                }
                this.insert(Tab { path: key, name: "Licenses".into(), body, pinned: true }, window, cx);
            })
            .ok();
        })
        .detach();
    }
}

fn severity(verdict: &Verdict) -> Option<DiagnosticSeverity> {
    match verdict {
        Verdict::Ok => None,
        Verdict::Caution(_) | Verdict::Unknown(_) => Some(DiagnosticSeverity::Warning),
        Verdict::Conflict(_) => Some(DiagnosticSeverity::Error),
    }
}

fn page(report: &Report) -> String {
    let own = report.own.as_deref();
    let mut rows: Vec<(Verdict, &licenses::Dep)> = report
        .deps
        .iter()
        .map(|d| (if d.shipped { verdict(own, d.license.as_deref()) } else { Verdict::Ok }, d))
        .collect();
    rows.sort_by(|(va, a), (vb, b)| vb.cmp(va).then_with(|| a.license.cmp(&b.license)).then_with(|| a.name.cmp(&b.name)));
    let problems = rows.iter().filter(|(v, _)| *v != Verdict::Ok).count();
    let mut out = format!(
        "# Dependency Licenses\n\nThis package: **{}** · {} dependencies · {} to check\n\n| Package | Version | License | Use | Note |\n|---|---|---|---|---|\n",
        own.unwrap_or("no license"),
        rows.len(),
        problems,
    );
    for (v, d) in rows {
        let note = match v {
            Verdict::Ok => String::new(),
            Verdict::Caution(why) | Verdict::Unknown(why) => format!("⚠️ {why}"),
            Verdict::Conflict(why) => format!("⛔ {why}"),
        };
        let usage = if d.shipped { "shipped" } else { "dev" };
        let license = d.license.as_deref().unwrap_or("—");
        out.push_str(&format!("| {} | {} | {} | {} | {} |\n", d.name, d.version, license, usage, note.replace('|', "\\|")));
    }
    out
}
