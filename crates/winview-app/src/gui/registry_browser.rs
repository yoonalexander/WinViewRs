use std::collections::BTreeMap;
use std::ops::Bound::{Excluded, Unbounded};

use eframe::egui;

use winview_core::model::{
    DecodingStatus, InspectionSnapshot, KeySnapshot, ReadError, ValueReadResult,
};

use super::diagnostics::ErrorPager;

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
    selected_value: Option<String>,
    error_pager: ErrorPager,
}

impl RegistryBrowser {
    fn reset_pages(&mut self) {
        self.key_page.reset();
        self.value_page.reset();
        self.selected_value = None;
        self.error_pager.reset();
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

        let (child_map, value_map, _errors) = if let Some(key) = current_key {
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
                    ui.ctx().request_repaint();
                }
                if ui
                    .add_enabled(child_has_next, egui::Button::new("Next"))
                    .clicked()
                {
                    if let Some(last) = child_entries.last() {
                        self.key_page.next(last.0.as_str());
                    }
                    ui.ctx().request_repaint();
                }
                ui.label(format!("Page {}", self.key_page.page_number()));
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
                    self.selected_value = None;
                    ui.ctx().request_repaint();
                }
                if ui
                    .add_enabled(val_has_next, egui::Button::new("Next"))
                    .clicked()
                {
                    if let Some(last) = val_entries.last() {
                        self.value_page.next(last.0.as_str());
                    }
                    self.selected_value = None;
                    ui.ctx().request_repaint();
                }
                ui.label(format!("Page {}", self.value_page.page_number()));
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
                        let selected = self.selected_value.as_deref() == Some(name.as_str());
                        if ui.selectable_label(selected, meta.name).clicked() {
                            self.selected_value = Some(name.to_string());
                        }
                        ui.label(meta.type_id);
                        ui.label(meta.payload_size);
                        ui.label(meta.status);
                        ui.end_row();
                    }
                });
        }

        if let Some(selected) = self.selected_value.clone() {
            match value_source.get(&selected) {
                Some(observation) => {
                    super::value_details::show_value_details(ui, &selected, observation);
                }
                None => {
                    self.selected_value = None;
                }
            }
        }

        if let Some(key) = current_key {
            ui.separator();

            self.error_pager.show(
                ui,
                "Selected key inspection error details",
                &key.read_errors,
            );
        } else {
            ui.small("Select a captured registry key to inspect its recorded errors.");
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
        fn populate_cursors(browser: &mut RegistryBrowser) {
            browser.key_page.next("key-a");
            browser.key_page.next("key-b");
            browser.value_page.next("value-a");
            browser.value_page.next("value-b");
        }

        fn assert_cursors_reset(browser: &RegistryBrowser) {
            assert!(browser.key_page.after.is_none());
            assert!(browser.key_page.history.is_empty());
            assert_eq!(browser.key_page.page_number(), 1);

            assert!(browser.value_page.after.is_none());
            assert!(browser.value_page.history.is_empty());
            assert_eq!(browser.value_page.page_number(), 1);
        }

        let mut browser = RegistryBrowser::default();

        assert_eq!(browser.domain, RegistryDomain::Bags);
        assert!(browser.path.is_empty());
        assert_cursors_reset(&browser);

        // Selecting the same domain preserves all state.
        browser.path.push("root".into());
        populate_cursors(&mut browser);

        let original_path = browser.path.clone();
        let key_after = browser.key_page.after.clone();
        let key_history = browser.key_page.history.clone();
        let value_after = browser.value_page.after.clone();
        let value_history = browser.value_page.history.clone();

        browser.select_domain(RegistryDomain::Bags);

        assert_eq!(browser.path, original_path);
        assert_eq!(browser.key_page.after, key_after);
        assert_eq!(browser.key_page.history, key_history);
        assert_eq!(browser.value_page.after, value_after);
        assert_eq!(browser.value_page.history, value_history);

        // Switching domains clears all navigation state.
        browser.select_domain(RegistryDomain::Streams);

        assert_eq!(browser.domain, RegistryDomain::Streams);
        assert!(browser.path.is_empty());
        assert_cursors_reset(&browser);

        // Entering a key resets both cursors.
        populate_cursors(&mut browser);
        browser.enter_key("root");

        assert_eq!(browser.path, vec!["root".to_string()]);
        assert_cursors_reset(&browser);

        // Entering a child also resets both cursors.
        populate_cursors(&mut browser);
        browser.enter_key(r"root\child");

        assert_eq!(
            browser.path,
            vec!["root".to_string(), r"root\child".to_string(),],
        );
        assert_cursors_reset(&browser);

        // Up removes exactly one path element.
        populate_cursors(&mut browser);
        browser.go_up();

        assert_eq!(browser.path, vec!["root".to_string()]);
        assert_cursors_reset(&browser);

        // Roots clears the entire path.
        populate_cursors(&mut browser);
        browser.go_to_roots();

        assert!(browser.path.is_empty());
        assert_cursors_reset(&browser);

        // Up at roots remains safe.
        browser.go_up();

        assert!(browser.path.is_empty());
        assert_cursors_reset(&browser);
    }

    #[test]
    fn registry_value_metadata_preserves_read_states() {
        use winview_core::model::{ObservationSource, RegistryValue};
        use winview_core::RootKey;

        fn observation(
            name: &str,
            type_id: u32,
            bytes: Vec<u8>,
            decoded: Option<RegistryValue>,
            status: DecodingStatus,
        ) -> ValueReadResult {
            ValueReadResult::Success(RawRegistryValue {
                key_path: r"Software\Test".into(),
                value_name: name.into(),
                root: RootKey::Hkcu,
                raw_type_id: type_id,
                raw_bytes: bytes,
                decoded,
                decoding_status: status,
                source: ObservationSource::HkcuBags,
            })
        }

        fn assert_metadata(
            name: &str,
            observation: &ValueReadResult,
            expected_name: &str,
            expected_type: &str,
            expected_size: &str,
            expected_status: &str,
        ) {
            let metadata = value_metadata(name, observation);

            assert_eq!(metadata.name, expected_name);
            assert_eq!(metadata.type_id, expected_type);
            assert_eq!(metadata.payload_size, expected_size);
            assert_eq!(metadata.status, expected_status);
        }

        let dword = observation(
            "Mode",
            4,
            0u32.to_le_bytes().to_vec(),
            Some(RegistryValue::Dword(0)),
            DecodingStatus::Decoded,
        );

        assert_metadata("Mode", &dword, "Mode", "4", "4 bytes", "Decoded");

        let empty_binary = observation(
            "",
            3,
            vec![],
            Some(RegistryValue::Binary(vec![])),
            DecodingStatus::Decoded,
        );

        assert_metadata("", &empty_binary, "(Default)", "3", "0 bytes", "Decoded");

        let unknown = observation(
            "Unknown",
            99,
            vec![1, 2, 3],
            None,
            DecodingStatus::UnknownType,
        );

        assert_metadata(
            "Unknown",
            &unknown,
            "Unknown",
            "99",
            "3 bytes",
            "Unknown registry type",
        );

        let malformed = observation(
            "Malformed",
            4,
            vec![1],
            None,
            DecodingStatus::Malformed {
                reason: "Invalid DWORD length".into(),
            },
        );

        assert_metadata(
            "Malformed",
            &malformed,
            "Malformed",
            "4",
            "1 bytes",
            "Malformed data",
        );

        for (error, expected_status) in [
            (
                ReadError::ValueNotFound {
                    name: "Missing".into(),
                },
                "Value not found",
            ),
            (
                ReadError::KeyNotFound {
                    path: r"Software\Test".into(),
                },
                "Key not found",
            ),
            (
                ReadError::AccessDenied {
                    path: r"Software\Test".into(),
                },
                "Access denied",
            ),
            (ReadError::Other("Injected error".into()), "Read failed"),
        ] {
            assert_metadata(
                "Missing",
                &ValueReadResult::Failure(error),
                "Missing",
                "Unavailable",
                "Unavailable",
                expected_status,
            );
        }

        let long_name = "한".repeat(161);

        let long_observation = observation(
            &long_name,
            4,
            1u32.to_le_bytes().to_vec(),
            Some(RegistryValue::Dword(1)),
            DecodingStatus::Decoded,
        );

        let metadata = value_metadata(&long_name, &long_observation);

        assert_eq!(metadata.name, format!("{}…", "한".repeat(160)),);

        assert_eq!(metadata.name.chars().count(), 161);
        assert_eq!(metadata.type_id, "4");
        assert_eq!(metadata.payload_size, "4 bytes");
        assert_eq!(metadata.status, "Decoded");

        // Metadata formatting must not alter captured data
        // or introduce additional inspection errors.
        let mut key = KeySnapshot::default();

        key.values.insert("Mode".into(), dword);
        key.values.insert("".into(), empty_binary);
        key.values.insert("Unknown".into(), unknown);
        key.values.insert("Malformed".into(), malformed);

        key.values.insert(
            "Missing".into(),
            ValueReadResult::Failure(ReadError::ValueNotFound {
                name: "Missing".into(),
            }),
        );

        let before = serde_json::to_string(&key).unwrap();

        for (name, observation) in &key.values {
            let _ = value_metadata(name, observation);
        }

        let after = serde_json::to_string(&key).unwrap();

        assert_eq!(before, after);
        assert!(key.read_errors.is_empty());
    }

    #[test]
    fn registry_diagnostics_preserve_scope_and_navigation() {
        use winview_core::inspection::count_snapshot_read_errors;
        use winview_core::model::InspectionError;

        fn error(root: winview_core::RootKey, path: &str, message: &str) -> InspectionError {
            InspectionError {
                key_path: path.into(),
                root,
                operation: "open_key".into(),
                error: message.into(),
            }
        }

        let mut snapshot = InspectionSnapshot::default();

        // Root key with enough errors for multiple diagnostic pages.
        let mut bag_root = KeySnapshot {
            read_errors: (0..30)
                .map(|i| {
                    error(
                        winview_core::RootKey::Hkcu,
                        "bagroot",
                        &format!("root error {i}"),
                    )
                })
                .collect(),
            ..Default::default()
        };
        bag_root.values.insert(
            "Missing".into(),
            ValueReadResult::Failure(ReadError::ValueNotFound {
                name: "Missing".into(),
            }),
        );

        // Nested child key with distinct errors, including an identical duplicate.
        let mut child = KeySnapshot::default();
        child.read_errors.push(error(
            winview_core::RootKey::Hkcu,
            r"bagroot\child",
            "Access is denied.",
        ));
        child.read_errors.push(error(
            winview_core::RootKey::Hkcu,
            r"bagroot\child",
            "Access is denied.",
        ));
        bag_root.subkeys.insert(r"bagroot\child".into(), child);
        snapshot.bags.insert("bagroot".into(), bag_root);

        // Separate HKLM and HKCU policy roots.
        let mut hklm_policy = KeySnapshot::default();
        hklm_policy
            .read_errors
            .push(error(winview_core::RootKey::Hklm, "HKLM-pol", "denied"));
        snapshot
            .policies
            .insert(r"HKLM\Software\Policies\Test".into(), hklm_policy);

        let mut hkcu_policy = KeySnapshot::default();
        hkcu_policy
            .read_errors
            .push(error(winview_core::RootKey::Hkcu, "HKCU-pol", "denied"));
        snapshot
            .policies
            .insert(r"Software\Policies\Test".into(), hkcu_policy);

        // Another registry domain.
        let mut stream_root = KeySnapshot::default();
        stream_root
            .read_errors
            .push(error(winview_core::RootKey::Hkcu, "streamroot", "failed"));
        snapshot.streams.insert("streamroot".into(), stream_root);

        let before = count_snapshot_read_errors(&snapshot);
        let before_json = serde_json::to_string(&snapshot).unwrap();

        // Exact selected key resolution returns the intended error slice.
        let bag_roots = RegistryDomain::Bags.roots(&snapshot);
        let resolved = resolve_key(bag_roots, &["bagroot".to_string()]).unwrap();
        assert_eq!(resolved.read_errors.len(), 30);

        let child_resolved = resolve_key(
            bag_roots,
            &["bagroot".to_string(), r"bagroot\child".to_string()],
        )
        .unwrap();
        // Identical recorded error entries are not deduplicated.
        assert_eq!(child_resolved.read_errors.len(), 2);
        assert_eq!(
            child_resolved.read_errors[0].key_path,
            child_resolved.read_errors[1].key_path
        );
        assert_eq!(
            child_resolved.read_errors[0].error,
            child_resolved.read_errors[1].error
        );

        // Parent and child diagnostics remain separate.
        assert_ne!(resolved.read_errors.len(), child_resolved.read_errors.len());

        // HKCU and HKLM diagnostics remain separate.
        let policy_roots = RegistryDomain::Policies.roots(&snapshot);
        let hklm =
            resolve_key(policy_roots, &[r"HKLM\Software\Policies\Test".to_string()]).unwrap();
        let hkcu = resolve_key(policy_roots, &[r"Software\Policies\Test".to_string()]).unwrap();
        assert_eq!(hklm.read_errors[0].root, winview_core::RootKey::Hklm);
        assert_eq!(hkcu.read_errors[0].root, winview_core::RootKey::Hkcu);

        let mut browser = RegistryBrowser::default();

        // Entering a key resets diagnostic pagination.
        browser.error_pager.select_page(1, 30);
        assert_eq!(browser.error_pager.page_range(30).start, 25);
        browser.enter_key("bagroot");
        assert_eq!(browser.error_pager.page_range(30).start, 0);

        // Up resets diagnostic pagination.
        browser.error_pager.select_page(1, 30);
        browser.go_up();
        assert_eq!(browser.error_pager.page_range(30).start, 0);

        // Roots resets diagnostic pagination.
        browser.error_pager.select_page(1, 30);
        browser.go_to_roots();
        assert_eq!(browser.error_pager.page_range(30).start, 0);

        // Changing domains resets diagnostic pagination.
        browser.error_pager.select_page(1, 30);
        browser.select_domain(RegistryDomain::Streams);
        assert_eq!(browser.error_pager.page_range(30).start, 0);

        // Selecting the same domain preserves diagnostic pagination.
        browser.error_pager.select_page(1, 30);
        browser.select_domain(RegistryDomain::Streams);
        assert_eq!(browser.error_pager.page_range(30).start, 25);

        // Changing only value selection preserves diagnostic pagination.
        browser.selected_value = Some("Missing".into());
        assert_eq!(browser.error_pager.page_range(30).start, 25);

        // Exercise the real renderer.
        let ctx = egui::Context::default();
        browser.select_domain(RegistryDomain::Bags);
        browser.enter_key("bagroot");
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            browser.show(ui, &snapshot);
        });
        output.textures_delta.clear();

        let after = count_snapshot_read_errors(&snapshot);
        let after_json = serde_json::to_string(&snapshot).unwrap();

        assert_eq!(before, after);
        assert_eq!(before_json, after_json);
    }
}
