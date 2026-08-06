use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{
    PyAny, PyBool, PyDict, PyDictMethods, PyFloat, PyInt, PyList, PyModule, PyString,
};

use logic_analyzer_protocol_decoders::sigrok_decoder::{
    InitialPin, SigrokAnnotationClassDescriptor, SigrokAnnotationRowDescriptor,
    SigrokCatalogDiagnostic, SigrokCatalogDiagnosticKind, SigrokCatalogEntry,
    SigrokCatalogSnapshot, SigrokDecoderChannelDescriptor, SigrokDecoderDescriptor,
    SigrokDecoderOptionDescriptor, SigrokOutputKind, SigrokScalarValue,
};

use super::bridge::DecoderBridge;
use super::python_error::format_python_error;
use super::python_host::{
    HostDecoder, OUTPUT_ANN, OUTPUT_BINARY, OUTPUT_LOGIC, OUTPUT_META, OUTPUT_PYTHON,
    SRD_CONF_SAMPLERATE, decoder_import_guard, install_sigrokdecode_module,
};

#[derive(Clone, Debug, PartialEq, Eq)]
enum SigrokSearchPathError {
    Missing,
    Unreadable(String),
}

trait SigrokSearchPathDiscovery: Send + Sync {
    fn normalize(&self, path: &Path) -> PathBuf;

    fn decoder_packages(&self, path: &Path) -> Result<Vec<PathBuf>, SigrokSearchPathError>;
}

trait SigrokPackageDiscovery: Send + Sync {
    fn discover(&self, decoder_root: &Path, id: &str) -> Result<SigrokDecoderDescriptor, String>;
}

struct SigrokDecoderCatalog {
    snapshots: Mutex<HashMap<Vec<PathBuf>, Arc<SigrokCatalogSnapshot>>>,
    scan_lock: Mutex<()>,
    search_paths: Arc<dyn SigrokSearchPathDiscovery>,
    packages: Arc<dyn SigrokPackageDiscovery>,
}

impl Default for SigrokDecoderCatalog {
    fn default() -> Self {
        Self::with_discovery(
            Arc::new(FilesystemSigrokSearchPathDiscovery),
            Arc::new(PythonSigrokPackageDiscovery),
        )
    }
}

impl SigrokDecoderCatalog {
    fn with_discovery(
        search_paths: Arc<dyn SigrokSearchPathDiscovery>,
        packages: Arc<dyn SigrokPackageDiscovery>,
    ) -> Self {
        Self {
            snapshots: Mutex::new(HashMap::new()),
            scan_lock: Mutex::new(()),
            search_paths,
            packages,
        }
    }

    #[cfg(test)]
    fn snapshot(&self, search_paths: &[PathBuf]) -> Arc<SigrokCatalogSnapshot> {
        let key = normalized_search_paths(search_paths, self.search_paths.as_ref());
        if let Some(snapshot) = self.snapshots.lock().unwrap().get(&key).cloned() {
            return snapshot;
        }
        let _scan = self.scan_lock.lock().unwrap();
        if let Some(snapshot) = self.snapshots.lock().unwrap().get(&key).cloned() {
            return snapshot;
        }
        self.store_scan(key)
    }

    fn refresh(&self, search_paths: &[PathBuf]) -> Arc<SigrokCatalogSnapshot> {
        let key = normalized_search_paths(search_paths, self.search_paths.as_ref());
        let _scan = self.scan_lock.lock().unwrap();
        self.store_scan(key)
    }

    fn store_scan(&self, key: Vec<PathBuf>) -> Arc<SigrokCatalogSnapshot> {
        let snapshot = Arc::new(scan_with_discovery(
            &key,
            self.search_paths.as_ref(),
            self.packages.as_ref(),
        ));
        self.snapshots.lock().unwrap().insert(key, snapshot.clone());
        snapshot
    }
}

struct FilesystemSigrokSearchPathDiscovery;

impl SigrokSearchPathDiscovery for FilesystemSigrokSearchPathDiscovery {
    fn normalize(&self, path: &Path) -> PathBuf {
        path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
    }

    fn decoder_packages(&self, path: &Path) -> Result<Vec<PathBuf>, SigrokSearchPathError> {
        if !path.exists() {
            return Err(SigrokSearchPathError::Missing);
        }
        let directory = std::fs::read_dir(path)
            .map_err(|error| SigrokSearchPathError::Unreadable(error.to_string()))?;
        let mut packages = directory
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|package| package.is_dir() && package.join("pd.py").is_file())
            .collect::<Vec<_>>();
        packages.sort();
        Ok(packages)
    }
}

struct PythonSigrokPackageDiscovery;

impl SigrokPackageDiscovery for PythonSigrokPackageDiscovery {
    fn discover(&self, decoder_root: &Path, id: &str) -> Result<SigrokDecoderDescriptor, String> {
        discover_sigrok_decoder(decoder_root, id)
    }
}

pub(crate) fn discover_sigrok_decoder(
    decoder_root: impl Into<PathBuf>,
    id: &str,
) -> Result<SigrokDecoderDescriptor, String> {
    let decoder_root = decoder_root.into();
    Python::initialize();
    let _import_guard = decoder_import_guard();
    Python::attach(|py| {
        install_sigrokdecode_module(py)?;
        let decoder_class = import_decoder(py, &decoder_root, id)?;
        let mut descriptor = descriptor_from_class(&decoder_class)?;
        let (_decoder, bridge) = start_decoder(py, &decoder_class, &descriptor)?;
        descriptor.registered_outputs = bridge
            .registrations()
            .into_iter()
            .filter_map(|registration| output_kind(registration.output_type))
            .collect();
        descriptor.package_fingerprint =
            package_fingerprint(&decoder_root.join(id)).map_err(PyValueError::new_err)?;
        PyResult::Ok(descriptor)
    })
    .map_err(|error| {
        format!(
            "could not discover Sigrok decoder '{id}':\n{}",
            format_python_error(error)
        )
    })
}

fn normalized_search_paths(
    search_paths: &[PathBuf],
    discovery: &dyn SigrokSearchPathDiscovery,
) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    search_paths
        .iter()
        .filter_map(|path| {
            let path = discovery.normalize(path);
            seen.insert(path.clone()).then_some(path)
        })
        .collect()
}

fn scan_with_discovery(
    search_paths: &[PathBuf],
    search_path_discovery: &dyn SigrokSearchPathDiscovery,
    package_discovery: &dyn SigrokPackageDiscovery,
) -> SigrokCatalogSnapshot {
    let mut snapshot = SigrokCatalogSnapshot::default();
    let mut decoder_ids = HashMap::<String, PathBuf>::new();
    for decoder_root in search_paths {
        let packages = match search_path_discovery.decoder_packages(decoder_root) {
            Ok(packages) => packages,
            Err(SigrokSearchPathError::Missing) => {
                snapshot.diagnostics.push(SigrokCatalogDiagnostic {
                    kind: SigrokCatalogDiagnosticKind::MissingSearchPath,
                    path: decoder_root.clone(),
                    decoder_id: None,
                    message: format!(
                        "Sigrok decoder search path does not exist: {}",
                        decoder_root.display()
                    ),
                });
                continue;
            }
            Err(SigrokSearchPathError::Unreadable(error)) => {
                snapshot.diagnostics.push(SigrokCatalogDiagnostic {
                    kind: SigrokCatalogDiagnosticKind::UnreadableSearchPath,
                    path: decoder_root.clone(),
                    decoder_id: None,
                    message: format!(
                        "Could not read Sigrok decoder search path {}: {error}",
                        decoder_root.display()
                    ),
                });
                continue;
            }
        };
        for package in packages {
            let Some(decoder_id) = package
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::to_owned)
            else {
                snapshot.diagnostics.push(SigrokCatalogDiagnostic {
                    kind: SigrokCatalogDiagnosticKind::InvalidDecoder,
                    path: package,
                    decoder_id: None,
                    message: "Sigrok decoder directory name is not valid UTF-8".to_owned(),
                });
                continue;
            };
            if let Some(first) = decoder_ids.get(&decoder_id) {
                snapshot.diagnostics.push(SigrokCatalogDiagnostic {
                    kind: SigrokCatalogDiagnosticKind::DuplicateDecoder,
                    path: package,
                    decoder_id: Some(decoder_id.clone()),
                    message: format!(
                        "Ignoring duplicate Sigrok decoder '{decoder_id}'; the earlier search path {} wins",
                        first.display()
                    ),
                });
                continue;
            }
            match package_discovery.discover(decoder_root, &decoder_id) {
                Ok(descriptor) => {
                    decoder_ids.insert(decoder_id, decoder_root.clone());
                    snapshot.entries.push(SigrokCatalogEntry {
                        decoder_root: decoder_root.clone(),
                        descriptor,
                    });
                }
                Err(message) => snapshot.diagnostics.push(SigrokCatalogDiagnostic {
                    kind: SigrokCatalogDiagnosticKind::InvalidDecoder,
                    path: package,
                    decoder_id: Some(decoder_id),
                    message,
                }),
            }
        }
    }
    snapshot
}

pub(crate) fn scan_catalog(directories: &[PathBuf]) -> SigrokCatalogSnapshot {
    (*SigrokDecoderCatalog::default().refresh(directories)).clone()
}

fn import_decoder<'py>(
    py: Python<'py>,
    decoder_root: &Path,
    id: &str,
) -> PyResult<Bound<'py, PyAny>> {
    let sys = PyModule::import(py, "sys")?;
    sys.setattr("dont_write_bytecode", true)?;
    let path: Bound<'_, PyList> = sys.getattr("path")?.cast_into()?;
    let decoder_root = decoder_root.to_str().ok_or_else(|| {
        PyValueError::new_err("the Sigrok decoder search path is not valid UTF-8")
    })?;
    path.call_method1("insert", (0, decoder_root))?;

    let modules: Bound<'_, PyDict> = sys.getattr("modules")?.cast_into()?;
    modules.del_item(id).ok();
    modules.del_item(format!("{id}.pd")).ok();

    PyModule::import(py, id)?.getattr("Decoder")
}

fn descriptor_from_class(decoder_class: &Bound<'_, PyAny>) -> PyResult<SigrokDecoderDescriptor> {
    let api_version = decoder_class.getattr("api_version")?.extract()?;
    if api_version != 3 {
        return Err(PyValueError::new_err(format!(
            "unsupported Sigrok decoder API version {api_version}"
        )));
    }

    Ok(SigrokDecoderDescriptor {
        api_version,
        id: string_attr(decoder_class, "id")?,
        name: string_attr(decoder_class, "name")?,
        long_name: string_attr(decoder_class, "longname")?,
        description: string_attr(decoder_class, "desc")?,
        license: string_attr(decoder_class, "license")?,
        inputs: string_sequence(&decoder_class.getattr("inputs")?)?,
        outputs: string_sequence(&decoder_class.getattr("outputs")?)?,
        tags: string_sequence(&decoder_class.getattr("tags")?)?,
        channels: decoder_class
            .getattr("channels")
            .ok()
            .map(|value| channels(&value))
            .transpose()?
            .unwrap_or_default(),
        optional_channels: decoder_class
            .getattr("optional_channels")
            .ok()
            .map(|value| channels(&value))
            .transpose()?
            .unwrap_or_default(),
        options: decoder_class
            .getattr("options")
            .ok()
            .map(|value| options(&value))
            .transpose()?
            .unwrap_or_default(),
        annotations: decoder_class
            .getattr("annotations")
            .ok()
            .map(|value| annotation_classes(&value))
            .transpose()?
            .unwrap_or_default(),
        annotation_rows: decoder_class
            .getattr("annotation_rows")
            .ok()
            .map(|value| annotation_rows(&value))
            .transpose()?
            .unwrap_or_default(),
        binary: decoder_class
            .getattr("binary")
            .ok()
            .map(|value| annotation_classes(&value))
            .transpose()?
            .unwrap_or_default(),
        logic_output_channels: decoder_class
            .getattr("logic_output_channels")
            .ok()
            .map(|value| channels(&value))
            .transpose()?
            .unwrap_or_default(),
        registered_outputs: Vec::new(),
        package_fingerprint: String::new(),
    })
}

fn start_decoder<'py>(
    py: Python<'py>,
    decoder_class: &Bound<'py, PyAny>,
    descriptor: &SigrokDecoderDescriptor,
) -> PyResult<(Bound<'py, PyAny>, Arc<DecoderBridge>)> {
    let decoder = decoder_class.call0()?;
    let channel_count = descriptor.channels.len() + descriptor.optional_channels.len();
    let (bridge, _outputs) = if channel_count == 0 && !descriptor.inputs.is_empty() {
        DecoderBridge::new_protocol(16)
    } else {
        DecoderBridge::new(vec![Some(InitialPin::Low); channel_count], 16)
            .map_err(|error| PyValueError::new_err(error.to_string()))?
    };
    decoder
        .cast::<HostDecoder>()?
        .borrow_mut()
        .attach(bridge.clone());
    let configured_options = PyDict::new(py);
    for option in &descriptor.options {
        set_scalar(&configured_options, &option.id, &option.default)?;
    }
    decoder.setattr("options", configured_options)?;
    decoder.setattr("samplenum", 0)?;
    decoder.setattr("matched", py.None())?;
    if decoder.hasattr("metadata")? {
        decoder.call_method1("metadata", (SRD_CONF_SAMPLERATE, 1_000_000_u64))?;
    }
    decoder.call_method0("start")?;
    Ok((decoder, bridge))
}

fn string_attr(object: &Bound<'_, PyAny>, name: &str) -> PyResult<String> {
    object.getattr(name)?.extract()
}

fn string_sequence(value: &Bound<'_, PyAny>) -> PyResult<Vec<String>> {
    value
        .try_iter()?
        .map(|item| item.and_then(|item| item.extract()))
        .collect()
}

fn channels(value: &Bound<'_, PyAny>) -> PyResult<Vec<SigrokDecoderChannelDescriptor>> {
    value
        .try_iter()?
        .map(|item| {
            let item = item?;
            if let Ok(item) = item.cast::<PyDict>() {
                return Ok(SigrokDecoderChannelDescriptor {
                    id: required_dict_item(item, "id")?.extract()?,
                    name: required_dict_item(item, "name")?.extract()?,
                    description: required_dict_item(item, "desc")?.extract()?,
                });
            }

            // Some upstream decoders build generated-logic channels with
            // compact `(id, name)` tuples instead of the dictionaries used
            // by regular input channels. libsigrokdecode accepts both forms.
            let fields = item
                .try_iter()?
                .map(|field| field.and_then(|field| field.extract::<String>()))
                .collect::<PyResult<Vec<_>>>()?;
            match fields.as_slice() {
                [id, name] => Ok(SigrokDecoderChannelDescriptor {
                    id: id.clone(),
                    name: name.clone(),
                    description: name.clone(),
                }),
                [id, name, description] => Ok(SigrokDecoderChannelDescriptor {
                    id: id.clone(),
                    name: name.clone(),
                    description: description.clone(),
                }),
                _ => Err(PyValueError::new_err(
                    "decoder channel must be a dictionary or an (id, name[, description]) sequence",
                )),
            }
        })
        .collect()
}

fn options(value: &Bound<'_, PyAny>) -> PyResult<Vec<SigrokDecoderOptionDescriptor>> {
    value
        .try_iter()?
        .map(|item| {
            let item = item?;
            let item = item.cast::<PyDict>()?;
            let values = item
                .get_item("values")?
                .map(|values| scalar_sequence(&values))
                .transpose()?
                .unwrap_or_default();
            Ok(SigrokDecoderOptionDescriptor {
                id: required_dict_item(item, "id")?.extract()?,
                description: required_dict_item(item, "desc")?.extract()?,
                default: scalar_value(&required_dict_item(item, "default")?)?,
                values,
            })
        })
        .collect()
}

fn annotation_classes(value: &Bound<'_, PyAny>) -> PyResult<Vec<SigrokAnnotationClassDescriptor>> {
    value
        .try_iter()?
        .map(|item| {
            let (id, description): (String, String) = item?.extract()?;
            Ok(SigrokAnnotationClassDescriptor { id, description })
        })
        .collect()
}

fn annotation_rows(value: &Bound<'_, PyAny>) -> PyResult<Vec<SigrokAnnotationRowDescriptor>> {
    value
        .try_iter()?
        .map(|item| {
            let (id, description, classes): (String, String, Vec<usize>) = item?.extract()?;
            Ok(SigrokAnnotationRowDescriptor {
                id,
                description,
                classes,
            })
        })
        .collect()
}

fn scalar_sequence(value: &Bound<'_, PyAny>) -> PyResult<Vec<SigrokScalarValue>> {
    value
        .try_iter()?
        .map(|item| item.and_then(|item| scalar_value(&item)))
        .collect()
}

fn scalar_value(value: &Bound<'_, PyAny>) -> PyResult<SigrokScalarValue> {
    if value.is_instance_of::<PyBool>() {
        Ok(SigrokScalarValue::Bool(value.extract()?))
    } else if value.is_instance_of::<PyInt>() {
        Ok(SigrokScalarValue::Integer(value.extract()?))
    } else if value.is_instance_of::<PyFloat>() {
        Ok(SigrokScalarValue::Float(value.extract()?))
    } else if value.is_instance_of::<PyString>() {
        Ok(SigrokScalarValue::String(value.extract()?))
    } else {
        Err(PyValueError::new_err(format!(
            "unsupported decoder option value type {}",
            value.get_type().name()?
        )))
    }
}

fn set_scalar(dict: &Bound<'_, PyDict>, key: &str, value: &SigrokScalarValue) -> PyResult<()> {
    match value {
        SigrokScalarValue::Bool(value) => dict.set_item(key, *value),
        SigrokScalarValue::Integer(value) => dict.set_item(key, *value),
        SigrokScalarValue::Float(value) => dict.set_item(key, *value),
        SigrokScalarValue::String(value) => dict.set_item(key, value),
    }
}

fn required_dict_item<'py>(dict: &Bound<'py, PyDict>, key: &str) -> PyResult<Bound<'py, PyAny>> {
    dict.get_item(key)?
        .ok_or_else(|| PyValueError::new_err(format!("missing required decoder field '{key}'")))
}

fn output_kind(output_type: i32) -> Option<SigrokOutputKind> {
    match output_type {
        OUTPUT_ANN => Some(SigrokOutputKind::Annotation),
        OUTPUT_BINARY => Some(SigrokOutputKind::Binary),
        OUTPUT_LOGIC => Some(SigrokOutputKind::GeneratedLogic),
        OUTPUT_META => Some(SigrokOutputKind::Metadata),
        OUTPUT_PYTHON => Some(SigrokOutputKind::ProtocolPacket),
        _ => None,
    }
}

fn package_fingerprint(package: &Path) -> Result<String, String> {
    fn collect_files(directory: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
        for entry in std::fs::read_dir(directory)
            .map_err(|error| format!("could not read {}: {error}", directory.display()))?
        {
            let path = entry
                .map_err(|error| format!("could not read {}: {error}", directory.display()))?
                .path();
            if path.is_dir() {
                if path.file_name().is_some_and(|name| name == "__pycache__") {
                    continue;
                }
                collect_files(&path, files)?;
            } else if path.is_file() {
                if matches!(
                    path.extension().and_then(|extension| extension.to_str()),
                    Some("pyc" | "pyo")
                ) {
                    continue;
                }
                files.push(path);
            }
        }
        Ok(())
    }

    let mut files = Vec::new();
    collect_files(package, &mut files)?;
    files.sort();
    let mut hasher = blake3::Hasher::new();
    for path in files {
        let relative = path.strip_prefix(package).unwrap_or(&path);
        hasher.update(relative.to_string_lossy().as_bytes());
        hasher.update(
            &std::fs::read(&path)
                .map_err(|error| format!("could not read {}: {error}", path.display()))?,
        );
    }
    Ok(hasher.finalize().to_hex().to_string())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::fs;
    use std::sync::{Arc, Barrier, Mutex, OnceLock};

    use super::*;

    #[derive(Default)]
    struct TestSearchPathDiscovery {
        packages: HashMap<PathBuf, Result<Vec<PathBuf>, SigrokSearchPathError>>,
    }

    impl TestSearchPathDiscovery {
        fn with_packages(mut self, root: &str, package_ids: &[&str]) -> Self {
            let root = PathBuf::from(root);
            self.packages.insert(
                root.clone(),
                Ok(package_ids.iter().map(|id| root.join(id)).collect()),
            );
            self
        }

        fn with_error(mut self, root: &str, error: SigrokSearchPathError) -> Self {
            self.packages.insert(PathBuf::from(root), Err(error));
            self
        }
    }

    impl SigrokSearchPathDiscovery for TestSearchPathDiscovery {
        fn normalize(&self, path: &Path) -> PathBuf {
            path.to_path_buf()
        }

        fn decoder_packages(&self, path: &Path) -> Result<Vec<PathBuf>, SigrokSearchPathError> {
            self.packages
                .get(path)
                .cloned()
                .unwrap_or(Err(SigrokSearchPathError::Missing))
        }
    }

    #[derive(Default)]
    struct TestPackageDiscovery {
        descriptors: HashMap<(PathBuf, String), Result<SigrokDecoderDescriptor, String>>,
        calls: Mutex<Vec<(PathBuf, String)>>,
    }

    impl TestPackageDiscovery {
        fn with_descriptor(mut self, root: &str, descriptor: SigrokDecoderDescriptor) -> Self {
            self.descriptors
                .insert((PathBuf::from(root), descriptor.id.clone()), Ok(descriptor));
            self
        }

        fn with_error(mut self, root: &str, id: &str, error: &str) -> Self {
            self.descriptors
                .insert((PathBuf::from(root), id.to_owned()), Err(error.to_owned()));
            self
        }

        fn calls(&self) -> Vec<(PathBuf, String)> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl SigrokPackageDiscovery for TestPackageDiscovery {
        fn discover(
            &self,
            decoder_root: &Path,
            id: &str,
        ) -> Result<SigrokDecoderDescriptor, String> {
            self.calls
                .lock()
                .unwrap()
                .push((decoder_root.to_path_buf(), id.to_owned()));
            self.descriptors
                .get(&(decoder_root.to_path_buf(), id.to_owned()))
                .cloned()
                .unwrap_or_else(|| Err(format!("no descriptor for {id}")))
        }
    }

    fn test_descriptor(id: &str, name: &str, license: &str) -> SigrokDecoderDescriptor {
        SigrokDecoderDescriptor {
            api_version: 3,
            id: id.to_owned(),
            name: name.to_owned(),
            long_name: name.to_owned(),
            description: "Test descriptor".into(),
            license: license.to_owned(),
            inputs: vec!["logic".into()],
            outputs: Vec::new(),
            tags: vec!["Test".into()],
            channels: Vec::new(),
            optional_channels: Vec::new(),
            options: Vec::new(),
            annotations: Vec::new(),
            annotation_rows: Vec::new(),
            binary: Vec::new(),
            logic_output_channels: Vec::new(),
            registered_outputs: Vec::new(),
            package_fingerprint: format!("fingerprint-{id}"),
        }
    }

    #[test]
    fn generated_fixture_decoder_can_be_discovered_and_started() {
        let directory = tempfile::tempdir().unwrap();
        write_fixture_decoder(directory.path(), "fixture_logic", "Fixture Logic", "mit");
        let _guard = python_test_lock().lock().unwrap();

        let descriptor = discover_sigrok_decoder(directory.path(), "fixture_logic").unwrap();

        assert_eq!(descriptor.api_version, 3);
        assert_eq!(descriptor.id, "fixture_logic");
        assert_eq!(descriptor.name, "Fixture Logic");
        assert_eq!(descriptor.license, "mit");
        assert_eq!(descriptor.inputs, ["logic"]);
        assert_eq!(descriptor.channels[0].id, "data");
    }

    #[test]
    fn generated_tuple_channels_are_discovered() {
        let directory = tempfile::tempdir().unwrap();
        write_tuple_channel_fixture(directory.path());
        let _guard = python_test_lock().lock().unwrap();

        let descriptor = discover_sigrok_decoder(directory.path(), "tuple_channels").unwrap();

        assert_eq!(descriptor.logic_output_channels.len(), 2);
        assert_eq!(descriptor.logic_output_channels[0].id, "p0");
        assert_eq!(descriptor.logic_output_channels[1].name, "P1");
    }

    #[test]
    fn concurrent_decoder_discovery_keeps_python_packages_registered() {
        let directory = tempfile::tempdir().unwrap();
        for id in ["fixture_a", "fixture_b"] {
            write_fixture_decoder(directory.path(), id, id, "mit");
            let package = directory.path().join(id);
            fs::write(package.join("lists.py"), "MARKER = 'loaded'\n").unwrap();
            let decoder = fs::read_to_string(package.join("pd.py")).unwrap();
            fs::write(
                package.join("pd.py"),
                format!("from .lists import MARKER\n{decoder}"),
            )
            .unwrap();
        }
        let decoder_root = directory.path().to_owned();

        let barrier = Arc::new(Barrier::new(5));
        let workers = (0..4)
            .map(|worker| {
                let barrier = Arc::clone(&barrier);
                let decoder_root = decoder_root.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    for iteration in 0..4 {
                        let id = if (worker + iteration) % 2 == 0 {
                            "fixture_a"
                        } else {
                            "fixture_b"
                        };
                        let descriptor = discover_sigrok_decoder(&decoder_root, id)?;
                        if descriptor.id != id {
                            return Err(format!(
                                "requested decoder {id}, discovered {}",
                                descriptor.id
                            ));
                        }
                    }
                    Ok::<_, String>(())
                })
            })
            .collect::<Vec<_>>();
        barrier.wait();

        for worker in workers {
            worker.join().expect("discovery worker panicked").unwrap();
        }
    }

    fn python_test_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn catalog_caches_ordered_search_results_and_reports_duplicates() {
        let first = PathBuf::from("virtual/first");
        let second = PathBuf::from("virtual/second");
        let search_paths = TestSearchPathDiscovery::default()
            .with_packages("virtual/first", &["fixture"])
            .with_packages("virtual/second", &["fixture"]);
        let packages = Arc::new(TestPackageDiscovery::default().with_descriptor(
            "virtual/first",
            test_descriptor("fixture", "First fixture", "mit"),
        ));
        let catalog =
            SigrokDecoderCatalog::with_discovery(Arc::new(search_paths), packages.clone());

        let snapshot = catalog.snapshot(&[first.clone(), second.clone()]);

        assert_eq!(snapshot.entries.len(), 1);
        assert_eq!(snapshot.entries[0].descriptor.name, "First fixture");
        assert_eq!(snapshot.entries[0].descriptor.license, "mit");
        assert!(snapshot.diagnostics.iter().any(|diagnostic| {
            diagnostic.kind == SigrokCatalogDiagnosticKind::DuplicateDecoder
                && diagnostic.decoder_id.as_deref() == Some("fixture")
        }));
        assert!(Arc::ptr_eq(
            &snapshot,
            &catalog.snapshot(&[first.clone(), second.clone()])
        ));
        assert!(!Arc::ptr_eq(&snapshot, &catalog.refresh(&[first, second])));
        assert_eq!(packages.calls().len(), 2);
    }

    #[test]
    fn catalog_keeps_missing_paths_as_structured_diagnostics() {
        let missing = PathBuf::from("virtual/missing");
        let unreadable = PathBuf::from("virtual/unreadable");
        let invalid = PathBuf::from("virtual/invalid");
        let search_paths = TestSearchPathDiscovery::default()
            .with_error("virtual/missing", SigrokSearchPathError::Missing)
            .with_error(
                "virtual/unreadable",
                SigrokSearchPathError::Unreadable("controlled read error".into()),
            )
            .with_packages("virtual/invalid", &["broken"]);
        let packages = TestPackageDiscovery::default().with_error(
            "virtual/invalid",
            "broken",
            "controlled package error",
        );
        let snapshot =
            SigrokDecoderCatalog::with_discovery(Arc::new(search_paths), Arc::new(packages))
                .snapshot(&[missing.clone(), unreadable.clone(), invalid.clone()]);

        assert!(snapshot.entries.is_empty());
        assert_eq!(snapshot.diagnostics.len(), 3);
        assert!(snapshot.diagnostics.iter().any(|diagnostic| {
            diagnostic.kind == SigrokCatalogDiagnosticKind::MissingSearchPath
                && diagnostic.path == missing
        }));
        assert!(snapshot.diagnostics.iter().any(|diagnostic| {
            diagnostic.kind == SigrokCatalogDiagnosticKind::UnreadableSearchPath
                && diagnostic.path == unreadable
                && diagnostic.message.contains("controlled read error")
        }));
        assert!(snapshot.diagnostics.iter().any(|diagnostic| {
            diagnostic.kind == SigrokCatalogDiagnosticKind::InvalidDecoder
                && diagnostic.path == invalid.join("broken")
                && diagnostic.message == "controlled package error"
        }));
    }

    #[test]
    fn generated_logic_channels_accept_sigrok_tuple_form() {
        let directory = tempfile::tempdir().unwrap();
        write_tuple_channel_fixture(directory.path());
        let _guard = python_test_lock().lock().unwrap();

        let snapshot = SigrokDecoderCatalog::default().refresh(&[directory.path().to_owned()]);

        assert!(
            snapshot.diagnostics.is_empty(),
            "{:?}",
            snapshot.diagnostics
        );
        let descriptor = &snapshot.entries[0].descriptor;
        assert_eq!(descriptor.logic_output_channels.len(), 2);
        assert_eq!(descriptor.logic_output_channels[0].id, "p0");
        assert_eq!(descriptor.logic_output_channels[0].name, "P0");
        assert_eq!(descriptor.logic_output_channels[0].description, "P0");
    }

    fn write_fixture_decoder(root: &Path, id: &str, name: &str, license: &str) {
        let package = root.join(id);
        fs::create_dir_all(&package).unwrap();
        fs::write(package.join("__init__.py"), "from .pd import Decoder\n").unwrap();
        fs::write(
            package.join("pd.py"),
            format!(
                r#"import sigrokdecode as srd

class Decoder(srd.Decoder):
    api_version = 3
    id = '{id}'
    name = '{name}'
    longname = '{name}'
    desc = 'Fixture decoder.'
    license = '{license}'
    inputs = ['logic']
    outputs = []
    tags = ['Test']
    channels = ({{'id': 'data', 'name': 'Data', 'desc': 'Data'}},)
    optional_channels = ()
    options = ()
    annotations = ()
    annotation_rows = ()
    binary = ()

    def metadata(self, key, value):
        self.samplerate = value

    def start(self):
        pass
"#
            ),
        )
        .unwrap();
    }

    fn write_tuple_channel_fixture(root: &Path) {
        let package = root.join("tuple_channels");
        fs::create_dir_all(&package).unwrap();
        fs::write(package.join("__init__.py"), "from .pd import Decoder\n").unwrap();
        fs::write(
            package.join("pd.py"),
            r#"import sigrokdecode as srd

class Decoder(srd.Decoder):
    api_version = 3
    id = 'tuple_channels'
    name = 'Tuple channels'
    longname = 'Tuple channels'
    desc = 'Tuple channel fixture.'
    license = 'mit'
    inputs = ['logic']
    outputs = []
    tags = ['Test']
    channels = ({'id': 'data', 'name': 'Data', 'desc': 'Data'},)
    optional_channels = ()
    options = ()
    annotations = ()
    annotation_rows = ()
    binary = ()
    logic_output_channels = (('p0', 'P0'), ('p1', 'P1'))

    def metadata(self, key, value):
        self.samplerate = value

    def start(self):
        self.register(srd.OUTPUT_LOGIC)
"#,
        )
        .unwrap();
    }
}
