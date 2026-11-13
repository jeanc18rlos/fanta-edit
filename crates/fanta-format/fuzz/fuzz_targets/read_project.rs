#![no_main]

use std::collections::BTreeMap;
use std::path::PathBuf;

use libfuzzer_sys::fuzz_target;

// A whole web project read from memory, so the SVG and CSS overlay, the node
// records and the FNX branches all run without touching the disk. The input is
// a list of files: a line `=== <path>` starts one, and its content runs to the
// next such line, less the final newline.
fuzz_target!(|data: &[u8]| {
    let mut files: BTreeMap<PathBuf, Vec<u8>> = BTreeMap::new();
    let mut current = None;
    for line in data.split_inclusive(|byte| *byte == b'\n') {
        if let Some(path) = line.strip_prefix(b"=== ") {
            let path = PathBuf::from(String::from_utf8_lossy(path).trim_end());
            files.entry(path.clone()).or_default();
            current = Some(path);
        } else if let Some(contents) = current.as_ref().and_then(|path| files.get_mut(path)) {
            contents.extend_from_slice(line);
        }
    }
    for contents in files.values_mut() {
        if contents.last() == Some(&b'\n') {
            contents.pop();
        }
    }
    fanta_format::read_project_files(&files).ok();
});
