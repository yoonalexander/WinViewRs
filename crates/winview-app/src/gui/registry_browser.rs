use std::collections::BTreeMap;
use std::ops::Bound::{Excluded, Unbounded};

use eframe::egui;

use winview_core::model::{
    DecodingStatus, InspectionSnapshot, KeySnapshot, ReadError, ValueReadResult,
};

const PAGE_SIZE: usize = 25;
const TEXT_PREVIEW_CHARS: usize = 160;

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
enum RegistryDomain {
    #[default]
    Bags,
    Streams,
    Clsid,
    Policies,
}

impl RegistryDomain {
    fn roots(self, snapshot: &InspectionSnapshot) -> &BTreeMap<String, KeySnapshot> {
        match self {
            Self::Bags => &snapshot.bags,
            Self::Streams => &snapshot.streams,
            Self::Clsid => &snapshot.clsid,
            Self::Policies => &snapshot.policies,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Bags => "Bags",
            Self::Streams => "Streams",
            Self::Clsid => "CLSID",
            Self::Policies => "Policies",
        }
    }
}

#[derive(Default)]
struct PageCursor {
    after: Option<String>,
    history: Vec<Option<String>>,
}

impl PageCursor {
    fn next(&mut self, last_key: &str) {
        self.history.push(self.after.take());
        self.after = Some(last_key.to_owned());
    }

    fn previous(&mut self) {
        if let Some(previous) = self.history.pop() {
            self.after = previous;
        }
    }

    fn page_number(&self) -> usize {
        self.history.len() + 1
    }

    fn reset(&mut self) {
        *self = Self::default();
    }
}

fn page_entries<'a, T>(
    map: &'a BTreeMap<String, T>,
    after: Option<&str>,
) -> (Vec<(&'a String, &'a T)>, bool) {
    let mut entries: Vec<_> = match after {
        Some(key) => map
            .range::<str, _>((Excluded(key), Unbounded))
            .take(PAGE_SIZE + 1)
            .collect(),

        None => map.iter().take(PAGE_SIZE + 1).collect(),
    };

    let has_next = entries.len() > PAGE_SIZE;

    entries.truncate(PAGE_SIZE);

    (entries, has_next)
}

fn resolve_key<'a>(
    roots: &'a BTreeMap<String, KeySnapshot>,
    path: &[String],
) -> Option<&'a KeySnapshot> {
    let (first, remaining) = path.split_first()?;

    let mut key = roots.get(first)?;

    for segment in remaining {
        key = key.subkeys.get(segment)?;
    }

    Some(key)
}

fn root_identity(root_key: &str) -> (&'static str, &str) {
    match root_key.strip_prefix("HKLM\\") {
        Some(path) => ("HKLM", path),
        None => ("HKCU", root_key),
    }
}

fn last_component(path: &str) -> &str {
    path.rsplit('\\').next().unwrap_or(path)
}
fn preview_text(value: &str) -> String {
    let mut chars = value.chars();
    let mut preview: String = chars.by_ref().take(TEXT_PREVIEW_CHARS).collect();
    if chars.next().is_some() {
        preview.push('…');
    }
    preview
}

#[derive(Debug, PartialEq, Eq)]
struct ValueMetadata {
    name: String,
    type_id: String,
    payload_size: String,
    status: &'static str,
}

fn value_metadata(name: &str, observation: &ValueReadResult) -> ValueMetadata {
    let name = if name.is_empty() {
        "(Default)".into()
    } else {
        preview_text(name)
    };

    let (type_id, payload_size) = match observation {
        ValueReadResult::Success(raw) => (
            raw.raw_type_id.to_string(),
            format!("{} bytes", raw.raw_bytes.len()),
        ),

        ValueReadResult::Failure(_) => ("Unavailable".into(), "Unavailable".into()),
    };

    ValueMetadata {
        name,
        type_id,
        payload_size,
        status: observation_status(observation),
    }
}
fn observation_status(observation: &ValueReadResult) -> &'static str {
    match observation {
        ValueReadResult::Failure(error) => match error {
            ReadError::ValueNotFound { .. } => "Value not found",
            ReadError::KeyNotFound { .. } => "Key not found",
            ReadError::AccessDenied { .. } => "Access denied",
            ReadError::Other(_) => "Read failed",
        },

        ValueReadResult::Success(raw) => match &raw.decoding_status {
            DecodingStatus::Decoded => "Decoded",
            DecodingStatus::Malformed { .. } => "Malformed data",
            DecodingStatus::UnknownType => "Unknown registry type",
        },
    }
}

#[derive(Default)]
pub(super) struct RegistryBrowser {
    domain: RegistryDomain,
    path: Vec<String>,
    key_page: PageCursor,
    value_page: PageCursor,
}

impl RegistryBrowser {
    fn reset_pages(&mut self) {
        self.key_page.reset();
        self.value_page.reset();
    }

    fn select_domain(&mut self, domain: RegistryDomain) {
        if self.domain != domain {
            self.domain = domain;
            self.path.clear();
            self.reset_pages();
        }
    }

    fn enter_key(&mut self, key: &str) {
        self.path.push(key.to_owned());
        self.reset_pages();
    }

    fn go_up(&mut self) {
        self.path.pop();
        self.reset_pages();
    }

    fn go_to_roots(&mut self) {
        self.path.clear();
        self.reset_pages();
    }

    pub(super) fn show(&mut self, ui: &mut egui::Ui, snapshot: &InspectionSnapshot) {
        ui.heading("Registry domains");

        ui.horizontal(|ui| {
            for d in [
                RegistryDomain::Bags,
                RegistryDomain::Streams,
                RegistryDomain::Clsid,
                RegistryDomain::Policies,
            ] {
                if ui.selectable_label(self.domain == d, d.label()).clicked() {
                    self.select_domain(d);
                }
            }
        });

        let roots = self.domain.roots(snapshot);

        let selected = resolve_key(roots, &self.path);
        if self.path.is_empty() {
            // roots
        } else if selected.is_none() {
            self.go_to_roots();
        }

        ui.horizontal(|ui| {
            if ui.button("Roots").clicked() {
                self.go_to_roots();
            }
            if ui
                .add_enabled(!self.path.is_empty(), egui::Button::new("Up"))
                .clicked()
            {
                self.go_up();
            }
        });

        let current_key = if self.path.is_empty() {
            None
        } else {
            resolve_key(roots, &self.path)
        };

        // hive identity
        let hive = self.path.first().map(|key| root_identity(key).0).unwrap_or(
            if self.domain == RegistryDomain::Policies {
                "HKLM/HKCU"
            } else {
                "HKCU"
            },
        );

        let path_display = self
            .path
            .last()
            .map(|key| {
                let path = if self.path.len() == 1 {
                    root_identity(key).1
                } else {
                    key.as_str()
                };

                preview_text(path)
            })
            .unwrap_or_else(|| "<roots>".into());
        ui.label(format!("Current hive: {}", hive));
        ui.label(format!("Registry path: {}", path_display));

        let (child_map, value_map, errors) = if let Some(key) = current_key {
            (&key.subkeys, &key.values, &key.read_errors)
        } else {
            // roots level: treat roots as children? We show root records as child list
            // We'll handle roots separately
            (roots, &BTreeMap::new(), &Vec::new())
        };

        // For roots level, show roots as children
        let at_roots = self.path.is_empty();

        if at_roots {
            ui.label(format!("Immediate child records: {}", roots.len()));
            ui.label("Value records: 0");
            ui.label(format!("Direct key errors: {}", 0));
        } else if let Some(key) = current_key {
            ui.label(format!("Immediate child records: {}", key.subkeys.len()));
            ui.label(format!("Value records: {}", key.values.len()));
            ui.label(format!("Direct key errors: {}", key.read_errors.len()));
        }

        // Pagination for child keys
        let child_source: &BTreeMap<String, KeySnapshot> = if at_roots { roots } else { child_map };

        let (child_entries, child_has_next) =
            page_entries(child_source, self.key_page.after.as_deref());
        ui.push_id("registry_key_pagination", |ui| {
            ui.horizontal(|ui| {
                let on_first = self.key_page.after.is_none() && self.key_page.history.is_empty();
                if ui
                    .add_enabled(!on_first, egui::Button::new("Previous"))
                    .clicked()
                {
                    self.key_page.previous();
                }
                ui.label(format!("Page {}", self.key_page.page_number()));
                if ui
                    .add_enabled(child_has_next, egui::Button::new("Next"))
                    .clicked()
                {
                    if let Some(last) = child_entries.last() {
                        self.key_page.next(last.0.as_str());
                    }
                }
            });
        });

        let (child_entries, _) = page_entries(child_source, self.key_page.after.as_deref());

        if child_entries.is_empty() {
            ui.label("No child key records were captured.");
        } else {
            let mut navigated = false;
            for (key_str, snap) in &child_entries {
                let preview = if at_roots {
                    let (hive, path) = root_identity(key_str);

                    format!("{hive}\\{}", preview_text(path))
                } else {
                    preview_text(last_component(key_str))
                };
                ui.push_id(key_str.as_str(), |ui| {
                    if ui
                        .button(format!(
                            "{}  (values: {}, subkeys: {}, errors: {})",
                            preview,
                            snap.values.len(),
                            snap.subkeys.len(),
                            snap.read_errors.len()
                        ))
                        .clicked()
                    {
                        navigated = true;
                    }
                });
                if navigated {
                    self.enter_key(key_str);
                    ui.ctx().request_repaint();
                    return;
                }
            }
        }

        // Values pagination
        let value_source: &BTreeMap<String, ValueReadResult> = if at_roots {
            // no values at roots
            &BTreeMap::new()
        } else {
            value_map
        };

        // Need a stable empty map reference; create static? Use a local empty
        // We'll handle empty case
        ui.separator();
        ui.heading("Values");
        let (val_entries, val_has_next) = if value_source.is_empty() && !at_roots {
            // empty map
            (Vec::new(), false)
        } else {
            page_entries(value_source, self.value_page.after.as_deref())
        };

        // empty map message
        if !at_roots {
            if let Some(key) = current_key {
                if key.values.is_empty() {
                    ui.label("No value records were captured for this key.");
                }
            }
        } else {
            ui.label("No value records at domain roots.");
        }

        ui.push_id("registry_value_pagination", |ui| {
            ui.horizontal(|ui| {
                let on_first =
                    self.value_page.after.is_none() && self.value_page.history.is_empty();
                if ui
                    .add_enabled(!on_first, egui::Button::new("Previous"))
                    .clicked()
                {
                    self.value_page.previous();
                }
                ui.label(format!("Page {}", self.value_page.page_number()));
                if ui
                    .add_enabled(val_has_next, egui::Button::new("Next"))
                    .clicked()
                {
                    if let Some(last) = val_entries.last() {
                        self.value_page.next(last.0.as_str());
                    }
                }
            });
        });

        let (val_entries, _) = page_entries(value_source, self.value_page.after.as_deref());

        if !val_entries.is_empty() {
            egui::Grid::new("registry_values")
                .num_columns(4)
                .striped(true)
                .show(ui, |ui| {
                    ui.label("Value name");
                    ui.label("Registry type ID");
                    ui.label("Payload size");
                    ui.label("Status");
                    ui.end_row();
                    for (name, obs) in &val_entries {
                        let meta = value_metadata(name, obs);
                        ui.label(meta.name);
                        ui.label(meta.type_id);
                        ui.label(meta.payload_size);
                        ui.label(meta.status);
                        ui.end_row();
                    }
                });
        }

        // errors display
        if !errors.is_empty() {
            ui.separator();
            ui.label(format!("Direct key errors: {}", errors.len()));
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    use winview_core::model::{InspectionSnapshot, KeySnapshot, RawRegistryValue, ValueReadResult};

    #[test]
    fn registry_domains_remain_separate() {
        let mut snap = InspectionSnapshot::default();
        let mut k = KeySnapshot::default();
        k.values.insert(
            "v".into(),
            ValueReadResult::Failure(ReadError::ValueNotFound { name: "v".into() }),
        );
        snap.bags.insert("bagroot".into(), k);
        snap.streams
            .insert("streamroot".into(), KeySnapshot::default());
        snap.clsid
            .insert("clsidroot".into(), KeySnapshot::default());
        snap.policies
            .insert("HKLM\\pol".into(), KeySnapshot::default());

        assert!(snap.bags.contains_key("bagroot"));
        assert!(!snap.streams.contains_key("bagroot"));
        assert_eq!(RegistryDomain::Bags.roots(&snap).len(), 1);
        assert_eq!(RegistryDomain::Streams.roots(&snap).len(), 1);
        assert_eq!(RegistryDomain::Clsid.roots(&snap).len(), 1);
        assert_eq!(RegistryDomain::Policies.roots(&snap).len(), 1);
        assert_eq!(root_identity("HKLM\\pol"), ("HKLM", "pol"));
        assert_eq!(
            root_identity("Software\\Policies"),
            ("HKCU", "Software\\Policies")
        );

        for (domain, expected) in [
            (RegistryDomain::Bags, "bagroot"),
            (RegistryDomain::Streams, "streamroot"),
            (RegistryDomain::Clsid, "clsidroot"),
            (RegistryDomain::Policies, "HKLM\\pol"),
        ] {
            let keys: Vec<_> = domain.roots(&snap).keys().map(String::as_str).collect();

            assert_eq!(keys, vec![expected]);
        }
    }

    #[test]
    fn registry_browser_preserves_hive_identity() {
        let mut snapshot = InspectionSnapshot::default();

        let root_path = r"Software\Policies\Test";
        let hklm_path = r"HKLM\Software\Policies\Test";
        let child_path = r"Software\Policies\Test\Child";

        for (root_key, value_name) in [(root_path, "UserValue"), (hklm_path, "SystemValue")] {
            let mut root = KeySnapshot::default();
            let mut child = KeySnapshot::default();

            child.values.insert(
                value_name.into(),
                ValueReadResult::Failure(ReadError::ValueNotFound {
                    name: value_name.into(),
                }),
            );

            root.subkeys.insert(child_path.into(), child);

            snapshot.policies.insert(root_key.into(), root);
        }

        let roots = RegistryDomain::Policies.roots(&snapshot);

        let user_path = vec![root_path.to_string(), child_path.to_string()];

        let system_path = vec![hklm_path.to_string(), child_path.to_string()];

        let user = resolve_key(roots, &user_path).unwrap();
        let system = resolve_key(roots, &system_path).unwrap();

        assert!(user.values.contains_key("UserValue"));
        assert!(!user.values.contains_key("SystemValue"));

        assert!(system.values.contains_key("SystemValue"));
        assert!(!system.values.contains_key("UserValue"));

        assert_eq!(root_identity(&user_path[0]), ("HKCU", root_path),);

        assert_eq!(root_identity(&system_path[0]), ("HKLM", root_path),);
    }

    #[test]
    fn registry_browser_resolves_full_path_keys() {
        let mut snap = InspectionSnapshot::default();
        let mut root = KeySnapshot::default();
        let mut child = KeySnapshot::default();
        let grand = KeySnapshot::default();
        child.subkeys.insert("root\\child\\grand".into(), grand);
        root.subkeys.insert("root\\child".into(), child);
        snap.bags.insert("root".into(), root);

        let roots = RegistryDomain::Bags.roots(&snap);
        assert!(resolve_key(roots, &["root".to_string()]).is_some());
        assert!(resolve_key(roots, &["root".to_string(), "root\\child".to_string()]).is_some());
        assert!(resolve_key(
            roots,
            &[
                "root".to_string(),
                "root\\child".to_string(),
                "root\\child\\grand".to_string()
            ]
        )
        .is_some());
        assert!(resolve_key(roots, &["nope".to_string()]).is_none());
        assert!(resolve_key(roots, &["root".to_string(), "missing".to_string()]).is_none());
    }

    #[test]
    fn registry_pagination_is_bounded_and_reversible() {
        let mut map: BTreeMap<String, KeySnapshot> = BTreeMap::new();
        for i in 0..51 {
            map.insert(format!("k{:02}", i), KeySnapshot::default());
        }
        // empty
        // not empty map test for empty separately
        let empty: BTreeMap<String, KeySnapshot> = BTreeMap::new();
        let (ee, ehn) = page_entries(&empty, None);
        assert!(ee.is_empty());
        assert!(!ehn);

        for n in [1usize, 25, 26, 51] {
            let mut m: BTreeMap<String, KeySnapshot> = BTreeMap::new();
            for i in 0..n {
                m.insert(format!("k{:02}", i), KeySnapshot::default());
            }
            // first page
            let (p1, hn1) = page_entries(&m, None);
            assert!(p1.len() <= 25);
            if n <= 25 {
                assert!(!hn1);
            } else {
                assert!(hn1);
            }
            // last key
            // walk pages
            let mut after: Option<String> = None;
            let mut collected: Vec<String> = Vec::new();
            loop {
                let (entries, has_next) = page_entries(&m, after.as_deref());
                for (k, _) in &entries {
                    collected.push((*k).clone());
                }
                if !has_next {
                    break;
                }
                after = entries.last().map(|e| e.0.clone());
            }
            assert_eq!(collected.len(), n);
            // no dup
            let mut sorted = collected.clone();
            sorted.sort();
            sorted.dedup();
            assert_eq!(sorted.len(), n);
        }

        // value map
        let mut vm: BTreeMap<String, ValueReadResult> = BTreeMap::new();
        for i in 0..26 {
            vm.insert(
                format!("v{}", i),
                ValueReadResult::Failure(ReadError::ValueNotFound {
                    name: format!("v{}", i),
                }),
            );
        }
        let (ve, vhn) = page_entries(&vm, None);
        assert_eq!(ve.len(), 25);
        assert!(vhn);

        let map: BTreeMap<String, KeySnapshot> = (0..51)
            .map(|i| (format!("k{i:02}"), KeySnapshot::default()))
            .collect();
        let mut cursor = PageCursor::default();
        let (first, has_next) = page_entries(&map, cursor.after.as_deref());
        assert_eq!(first.len(), 25);
        assert_eq!(cursor.page_number(), 1);
        assert!(has_next);
        cursor.next(first.last().unwrap().0);
        let (second, has_next) = page_entries(&map, cursor.after.as_deref());
        assert_eq!(second.len(), 25);
        assert_eq!(second[0].0.as_str(), "k25");
        assert_eq!(cursor.page_number(), 2);
        assert!(has_next);
        cursor.next(second.last().unwrap().0);
        let (third, has_next) = page_entries(&map, cursor.after.as_deref());
        assert_eq!(third.len(), 1);
        assert_eq!(third[0].0.as_str(), "k50");
        assert_eq!(cursor.page_number(), 3);
        assert!(!has_next);
        cursor.previous();
        assert_eq!(cursor.page_number(), 2);
        let (returned, _) = page_entries(&map, cursor.after.as_deref());
        assert_eq!(
            returned.iter().map(|e| e.0).collect::<Vec<_>>(),
            second.iter().map(|e| e.0).collect::<Vec<_>>()
        );
        cursor.previous();
        assert_eq!(cursor.page_number(), 1);
        assert!(cursor.after.is_none());
        cursor.next(first.last().unwrap().0);
        cursor.reset();
        assert_eq!(cursor.page_number(), 1);
        assert!(cursor.after.is_none());
        assert!(cursor.history.is_empty());
    }

    #[test]
    fn registry_navigation_resets_pagination() {
        let mut b = RegistryBrowser::default();
        assert_eq!(b.domain, RegistryDomain::Bags);
        b.key_page.after = Some("x".into());
        b.value_page.after = Some("y".into());
        b.path.push("p".into());

        b.select_domain(RegistryDomain::Streams);
        assert!(b.path.is_empty());
        assert!(b.key_page.after.is_none());
        assert!(b.value_page.after.is_none());

        // same domain preserves
        b.path.push("q".into());
        b.key_page.after = Some("z".into());
        let after_before = b.key_page.after.clone();
        b.select_domain(RegistryDomain::Streams);
        assert_eq!(b.key_page.after, after_before);

        // enter key resets
        b.key_page.after = Some("z".into());
        b.value_page.after = Some("w".into());
        b.enter_key("k");
        assert!(b.key_page.after.is_none());
        assert!(b.value_page.after.is_none());

        // up removes one
        b.go_to_roots();
        b.path.push("a".into());
        b.path.push("b".into());
        b.go_up();
        assert_eq!(b.path, vec!["a".to_string()]);

        b.go_to_roots();
        assert!(b.path.is_empty());

        // up at roots safe
        b.go_up();
        assert!(b.path.is_empty());
    }

    #[test]
    fn registry_value_metadata_preserves_read_states() {
        assert_eq!(
            observation_status(&ValueReadResult::Success(RawRegistryValue {
                key_path: "k".into(),
                value_name: "".into(),
                root: winview_core::RootKey::Hkcu,
                raw_type_id: 4,
                raw_bytes: vec![],
                decoded: Some(winview_core::model::RegistryValue::Dword(0)),
                decoding_status: DecodingStatus::Decoded,
                source: winview_core::model::ObservationSource::HkcuBags,
            })),
            "Decoded"
        );

        assert_eq!(
            observation_status(&ValueReadResult::Success(RawRegistryValue {
                key_path: "k".into(),
                value_name: "".into(),
                root: winview_core::RootKey::Hkcu,
                raw_type_id: 99,
                raw_bytes: vec![],
                decoded: None,
                decoding_status: DecodingStatus::UnknownType,
                source: winview_core::model::ObservationSource::HkcuBags,
            })),
            "Unknown registry type"
        );

        assert_eq!(
            observation_status(&ValueReadResult::Success(RawRegistryValue {
                key_path: "k".into(),
                value_name: "".into(),
                root: winview_core::RootKey::Hkcu,
                raw_type_id: 4,
                raw_bytes: vec![1],
                decoded: None,
                decoding_status: DecodingStatus::Malformed { reason: "x".into() },
                source: winview_core::model::ObservationSource::HkcuBags,
            })),
            "Malformed data"
        );

        assert_eq!(
            observation_status(&ValueReadResult::Failure(ReadError::ValueNotFound {
                name: "v".into()
            })),
            "Value not found"
        );
        assert_eq!(
            observation_status(&ValueReadResult::Failure(ReadError::KeyNotFound {
                path: "p".into()
            })),
            "Key not found"
        );
        assert_eq!(
            observation_status(&ValueReadResult::Failure(ReadError::AccessDenied {
                path: "p".into()
            })),
            "Access denied"
        );
        assert_eq!(
            observation_status(&ValueReadResult::Failure(ReadError::Other("e".into()))),
            "Read failed"
        );
    }
}
