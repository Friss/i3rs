//! Project save/load: wraps a workspace with a reusable session set.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::workspace::{PanelConfig, WorkspaceFile};

const PROJECT_FILE_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
pub struct ProjectFile {
    #[serde(default = "default_project_file_version")]
    pub version: u32,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub sessions: Vec<ProjectSessionEntry>,
    #[serde(default)]
    pub active_session_path: Option<String>,
    pub workspace: WorkspaceFile,
}

#[derive(Serialize, Deserialize)]
pub struct ProjectSessionEntry {
    pub path: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub notes: String,
}

fn default_project_file_version() -> u32 {
    PROJECT_FILE_VERSION
}

pub fn project_name_from_path(path: &Path) -> String {
    path.file_stem()
        .map(|stem| stem.to_string_lossy().to_string())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "Race Weekend".into())
}

impl ProjectFile {
    pub fn from_workspace(
        name: String,
        mut workspace: WorkspaceFile,
        session_entries: &[ProjectSessionEntry],
        active_session_path: Option<&Path>,
        project_path: &Path,
    ) -> Self {
        let base_dir = project_path.parent().unwrap_or_else(|| Path::new("."));
        relativize_workspace_paths(&mut workspace, base_dir);

        let mut sessions = Vec::new();
        for session in session_entries {
            push_unique_session(
                &mut sessions,
                ProjectSessionEntry {
                    path: relativize_path(Path::new(&session.path), base_dir),
                    label: session.label.clone(),
                    notes: session.notes.clone(),
                },
            );
        }

        Self {
            version: PROJECT_FILE_VERSION,
            name,
            sessions,
            active_session_path: active_session_path.map(|path| relativize_path(path, base_dir)),
            workspace,
        }
    }

    pub fn effective_name(&self, project_path: &Path) -> String {
        if self.name.trim().is_empty() {
            project_name_from_path(project_path)
        } else {
            self.name.clone()
        }
    }
}

pub fn resolve_project_paths(project: &mut ProjectFile, project_path: &Path) {
    let base_dir = project_path.parent().unwrap_or_else(|| Path::new("."));

    for session in &mut project.sessions {
        session.path = resolve_path_string(&session.path, base_dir);
    }
    if let Some(active_path) = &mut project.active_session_path {
        *active_path = resolve_path_string(active_path, base_dir);
    }

    resolve_workspace_paths(&mut project.workspace, base_dir);
}

pub fn collect_session_paths(workspace: &WorkspaceFile) -> Vec<PathBuf> {
    let mut paths = Vec::new();

    if let Some(path) = workspace.last_file_path.as_ref() {
        push_unique_path(&mut paths, PathBuf::from(path));
    }

    for worksheet in &workspace.worksheets {
        for panel in &worksheet.panels {
            if let PanelConfig::Graph(graph) = panel {
                for overlay in &graph.overlays {
                    if let Some(path) = overlay.file_path.as_ref() {
                        push_unique_path(&mut paths, PathBuf::from(path));
                    }
                }
            }
        }
    }

    paths
}

fn relativize_workspace_paths(workspace: &mut WorkspaceFile, base_dir: &Path) {
    if let Some(path) = workspace.last_file_path.as_mut() {
        *path = relativize_path(Path::new(path), base_dir);
    }

    for worksheet in &mut workspace.worksheets {
        for panel in &mut worksheet.panels {
            if let PanelConfig::Graph(graph) = panel {
                for overlay in &mut graph.overlays {
                    if let Some(path) = overlay.file_path.as_mut() {
                        *path = relativize_path(Path::new(path), base_dir);
                    }
                }
            }
        }
    }
}

fn resolve_workspace_paths(workspace: &mut WorkspaceFile, base_dir: &Path) {
    if let Some(path) = workspace.last_file_path.as_mut() {
        *path = resolve_path_string(path, base_dir);
    }

    for worksheet in &mut workspace.worksheets {
        for panel in &mut worksheet.panels {
            if let PanelConfig::Graph(graph) = panel {
                for overlay in &mut graph.overlays {
                    if let Some(path) = overlay.file_path.as_mut() {
                        *path = resolve_path_string(path, base_dir);
                    }
                }
            }
        }
    }
}

fn relativize_path(path: &Path, base_dir: &Path) -> String {
    if let Ok(relative) = path.strip_prefix(base_dir)
        && !relative.as_os_str().is_empty()
    {
        return relative.to_string_lossy().replace('\\', "/");
    }

    path.to_string_lossy().to_string()
}

fn resolve_path_string(path: &str, base_dir: &Path) -> String {
    let path = PathBuf::from(path);
    let resolved = if path.is_absolute() {
        path
    } else {
        base_dir.join(path)
    };
    resolved.to_string_lossy().to_string()
}

fn push_unique_session(entries: &mut Vec<ProjectSessionEntry>, entry: ProjectSessionEntry) {
    if !entries.iter().any(|existing| existing.path == entry.path) {
        entries.push(entry);
    }
}

fn push_unique_path(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if !paths.iter().any(|existing| existing == &path) {
        paths.push(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::{
        GraphOverlayConfig, GraphPanelConfig, PanelConfig, WorksheetConfig, WorkspaceFile,
    };

    fn sample_workspace() -> WorkspaceFile {
        WorkspaceFile {
            worksheets: vec![WorksheetConfig {
                name: "Analysis".into(),
                panels: vec![PanelConfig::Graph(GraphPanelConfig {
                    id: 1,
                    title: "Graph 1".into(),
                    channel_names: vec![],
                    colors: vec![],
                    tile_groups: vec![],
                    graph_mode: "Tiled".into(),
                    x_axis_mode: "Time".into(),
                    reference_lap: None,
                    overlays: vec![
                        GraphOverlayConfig {
                            file_path: Some("/tmp/weekend/session-02.ld".into()),
                            lap_index: 1,
                            manual_offset: 0.0,
                            stretch_to_reference: false,
                        },
                        GraphOverlayConfig {
                            file_path: Some("/tmp/weekend/session-02.ld".into()),
                            lap_index: 2,
                            manual_offset: 0.0,
                            stretch_to_reference: true,
                        },
                    ],
                    embedded_gauges: vec![],
                    is_math: vec![],
                    display_transforms: vec![],
                })],
            }],
            active_worksheet: 0,
            last_file_path: Some("/tmp/weekend/session-01.ld".into()),
            math_channels: vec![],
            channel_aliases: vec![],
            sectors: vec![],
            reference_lap: None,
        }
    }

    #[test]
    fn collect_session_paths_deduplicates_main_and_overlay_sessions() {
        let workspace = sample_workspace();
        let paths = collect_session_paths(&workspace);

        assert_eq!(
            paths,
            vec![
                PathBuf::from("/tmp/weekend/session-01.ld"),
                PathBuf::from("/tmp/weekend/session-02.ld"),
            ]
        );
    }

    #[test]
    fn project_paths_round_trip_relative_to_project_file() {
        let workspace = sample_workspace();
        let project_path = PathBuf::from("/tmp/weekend/race-weekend.i3rsproj");
        let mut project = ProjectFile::from_workspace(
            "Race Weekend".into(),
            workspace,
            &[
                ProjectSessionEntry {
                    path: "/tmp/weekend/session-01.ld".into(),
                    label: "FP1".into(),
                    notes: "Opening baseline".into(),
                },
                ProjectSessionEntry {
                    path: "/tmp/weekend/session-02.ld".into(),
                    label: "Qualifying".into(),
                    notes: String::new(),
                },
            ],
            Some(Path::new("/tmp/weekend/session-01.ld")),
            &project_path,
        );

        assert_eq!(
            project.active_session_path.as_deref(),
            Some("session-01.ld")
        );
        assert_eq!(project.sessions[1].path, "session-02.ld");
        assert_eq!(project.sessions[0].label, "FP1");
        assert_eq!(
            project.workspace.last_file_path.as_deref(),
            Some("session-01.ld")
        );

        resolve_project_paths(&mut project, &project_path);

        assert_eq!(
            project.active_session_path.as_deref(),
            Some("/tmp/weekend/session-01.ld")
        );
        assert_eq!(
            project.workspace.last_file_path.as_deref(),
            Some("/tmp/weekend/session-01.ld")
        );
        let overlay_path = project.workspace.worksheets[0]
            .panels
            .iter()
            .find_map(|panel| match panel {
                PanelConfig::Graph(graph) => graph.overlays[0].file_path.clone(),
                _ => None,
            });
        assert_eq!(overlay_path.as_deref(), Some("/tmp/weekend/session-02.ld"));
    }
}
