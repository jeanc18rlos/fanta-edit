//! Filling templates from data: one row of values sets text variables and
//! placeholder images, and a batch of rows exports one image per row. Image
//! values go through the session's [`ImageFetcher`], which by default decodes
//! `data:` URIs only, so template data reads no files and no network unless
//! the caller sets a fetcher that does.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, bail};
use fanta_doc::{AssetId, NodeId};
use fanta_headless::assets::{MAX_IMAGE_SOURCE_BYTES, decode_image_source};

use crate::{ApiError, DesignSession, ExportEncoding, ExportSizing, ExportedImage, Result, Role};

/// How long [`HttpFetcher`] waits for one image.
const FETCH_TIMEOUT: Duration = Duration::from_secs(30);

/// Turns an image value from template data into encoded image bytes.
pub trait ImageFetcher: Send + Sync {
    fn fetch(&self, source: &str) -> anyhow::Result<Vec<u8>>;
}

/// Decodes base64 `data:` URIs, and bare base64, and fetches nothing.
#[derive(Debug, Default, Clone, Copy)]
pub struct DataUriFetcher;

impl ImageFetcher for DataUriFetcher {
    fn fetch(&self, source: &str) -> anyhow::Result<Vec<u8>> {
        let source = source.trim();
        if source.contains("://") {
            bail!("`{source}` is not a data: URI, and this session fetches nothing else");
        }
        decode_image_source(source)
    }
}

/// Reads images from files under one directory, by paths relative to it,
/// and decodes `data:` URIs. A path that leads out of the directory, through
/// `..`, an absolute path or a symlink, is refused.
#[derive(Debug, Clone)]
pub struct DirectoryFetcher {
    root: PathBuf,
}

impl DirectoryFetcher {
    pub fn new(root: &Path) -> anyhow::Result<Self> {
        let root = root
            .canonicalize()
            .with_context(|| format!("opening {}", root.display()))?;
        Ok(Self { root })
    }
}

impl ImageFetcher for DirectoryFetcher {
    fn fetch(&self, source: &str) -> anyhow::Result<Vec<u8>> {
        let source = source.trim();
        if source.starts_with("data:") {
            return DataUriFetcher.fetch(source);
        }
        let path = self
            .root
            .join(source.strip_prefix("file://").unwrap_or(source))
            .canonicalize()
            .with_context(|| format!("no file at `{source}`"))?;
        if !path.starts_with(&self.root) {
            bail!("`{source}` is outside {}", self.root.display());
        }
        let size = std::fs::metadata(&path)
            .with_context(|| format!("reading `{source}`"))?
            .len();
        if size > MAX_IMAGE_SOURCE_BYTES as u64 {
            bail!("`{source}` is {size} bytes; the limit is {MAX_IMAGE_SOURCE_BYTES}");
        }
        std::fs::read(&path).with_context(|| format!("reading `{source}`"))
    }
}

/// Fetches `https` URLs up to the image size limit, and decodes `data:`
/// URIs. Plain `http` and a host allowlist are opt in, and redirects are not
/// followed, so none can lead past the allowlist. Give it an allowlist when
/// rows come from untrusted users: otherwise it fetches any host a row names,
/// including ones on its own network.
#[derive(Clone)]
pub struct HttpFetcher {
    agent: ureq::Agent,
    allow_plain_http: bool,
    allowed_hosts: Option<BTreeSet<String>>,
}

impl HttpFetcher {
    pub fn new() -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(FETCH_TIMEOUT))
            .max_redirects(0)
            .build()
            .into();
        Self {
            agent,
            allow_plain_http: false,
            allowed_hosts: None,
        }
    }

    /// Fetch plain `http` URLs too.
    pub fn allow_plain_http(mut self) -> Self {
        self.allow_plain_http = true;
        self
    }

    /// Fetch only from `hosts`.
    pub fn allow_hosts<I, S>(mut self, hosts: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.allowed_hosts = Some(
            hosts
                .into_iter()
                .map(|host| host.into().to_ascii_lowercase())
                .collect(),
        );
        self
    }
}

impl Default for HttpFetcher {
    fn default() -> Self {
        Self::new()
    }
}

impl ImageFetcher for HttpFetcher {
    fn fetch(&self, source: &str) -> anyhow::Result<Vec<u8>> {
        let source = source.trim();
        if source.starts_with("data:") {
            return DataUriFetcher.fetch(source);
        }
        let uri: ureq::http::Uri = source
            .parse()
            .with_context(|| format!("`{source}` is not a URL"))?;
        match uri.scheme_str() {
            Some("https") => {}
            Some("http") if self.allow_plain_http => {}
            _ => bail!("`{source}` is not an https URL"),
        }
        let host = uri
            .host()
            .with_context(|| format!("`{source}` names no host"))?
            .to_ascii_lowercase();
        if let Some(allowed) = &self.allowed_hosts
            && !allowed.contains(&host)
        {
            bail!("{host} is not an allowed host");
        }
        let mut response = self
            .agent
            .get(source)
            .call()
            .with_context(|| format!("fetching {source}"))?;
        if response.status() != 200 {
            bail!("fetching {source} answered {}", response.status());
        }
        let mut bytes = Vec::new();
        response
            .body_mut()
            .as_reader()
            .take(MAX_IMAGE_SOURCE_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .with_context(|| format!("reading {source}"))?;
        if bytes.len() > MAX_IMAGE_SOURCE_BYTES {
            bail!("{source} is over the {MAX_IMAGE_SOURCE_BYTES} byte limit");
        }
        Ok(bytes)
    }
}

/// What one key of a row sets.
enum Assignment<'a> {
    Text {
        key: &'a str,
        value: &'a str,
    },
    Images {
        placeholders: Vec<NodeId>,
        bytes: Vec<u8>,
    },
}

impl DesignSession {
    /// Fetch the image values of template data through `fetcher` from now on.
    pub fn set_image_fetcher(&mut self, fetcher: impl ImageFetcher + 'static) {
        self.image_fetcher = Arc::new(fetcher);
    }

    /// Fill the template with one row of data. A key sets the text variable
    /// it names; failing that, every placeholder named after the key shows
    /// the image its value fetches; failing that, the key becomes a new text
    /// variable. Images are fetched before anything changes, and when any key
    /// fails the whole row is rolled back.
    pub fn fill_template(&mut self, row: &BTreeMap<String, String>) -> Result<()> {
        if self.role() == Role::Viewer {
            return Err(ApiError::ViewOnly);
        }
        let mut assignments = Vec::new();
        for (key, value) in row {
            let key = key.trim();
            let names_variable =
                fanta_doc::text_token_variable(&self.doc().variables, key).is_some();
            let placeholders = if names_variable {
                Vec::new()
            } else {
                self.placeholders_named(key)
            };
            if placeholders.is_empty() {
                assignments.push(Assignment::Text { key, value });
                continue;
            }
            let bytes = self.image_fetcher.fetch(value).map_err(|error| {
                ApiError::Engine(error.context(format!("fetching the image for `{key}`")))
            })?;
            assignments.push(Assignment::Images {
                placeholders,
                bytes,
            });
        }
        let savepoint = crate::open_pending_step(self.document_mut().doc_mut());
        let mut ingested = Vec::new();
        if let Err(error) = self.assign(assignments, &mut ingested) {
            self.document_mut().roll_back_to(savepoint)?;
            for asset in ingested {
                self.document_mut().remove_image(asset);
            }
            return Err(error);
        }
        Ok(())
    }

    /// Fill a copy of the design with each row and export `node` from it:
    /// one result per row, in order. The session itself does not change.
    pub fn batch_export(
        &self,
        rows: &[BTreeMap<String, String>],
        node: NodeId,
        encoding: ExportEncoding,
        sizing: ExportSizing,
    ) -> Vec<Result<ExportedImage>> {
        rows.iter()
            .map(|row| {
                let mut copy = self.copy();
                copy.fill_template(row)?;
                copy.export(node, encoding, sizing)
            })
            .collect()
    }

    /// Apply `assignments`, collecting the image assets they added so a
    /// failed row can remove them again.
    fn assign(
        &mut self,
        assignments: Vec<Assignment<'_>>,
        ingested: &mut Vec<AssetId>,
    ) -> Result<()> {
        for assignment in assignments {
            match assignment {
                Assignment::Text { key, value } => self.set_text_variable(key, value)?,
                Assignment::Images {
                    placeholders,
                    bytes,
                } => {
                    for placeholder in placeholders {
                        ingested.extend(self.swap_placeholder_image(placeholder, bytes.clone())?);
                    }
                }
            }
        }
        Ok(())
    }

    fn placeholders_named(&self, name: &str) -> Vec<NodeId> {
        self.find(|_, node| {
            node.name == name
                && node
                    .editing
                    .as_ref()
                    .is_some_and(|rules| rules.placeholder.is_some())
        })
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read as _, Write as _};
    use std::net::TcpListener;

    use base64::Engine as _;
    use fanta_design_ops::{DesignOp, DesignVariableType};
    use fanta_doc::{NodeData, Placeholder};

    use super::*;
    use crate::NodeKind;

    fn png(width: u32, height: u32, rgba: [u8; 4]) -> Vec<u8> {
        let mut bytes = Vec::new();
        image::RgbaImage::from_pixel(width, height, image::Rgba(rgba))
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .expect("encode png");
        bytes
    }

    fn data_uri(bytes: &[u8]) -> String {
        format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(bytes)
        )
    }

    fn row(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
        entries
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    }

    fn bitmap_asset(session: &DesignSession, node: NodeId) -> AssetId {
        match &session.node(node).expect("node").data {
            NodeData::Bitmap(bitmap) => bitmap.asset,
            _ => panic!("a bitmap"),
        }
    }

    fn shown_text(session: &DesignSession, node: NodeId) -> String {
        let doc = session.doc();
        match fanta_doc::displayed_text_node(&doc.scene, &doc.variables, &doc.active_modes, node)
            .map(|node| node.data)
        {
            Some(NodeData::Text(text)) => text.content,
            _ => panic!("a text node"),
        }
    }

    /// A page with `{{name}}` text and an image placeholder named "photo".
    fn template() -> (DesignSession, NodeId, NodeId) {
        let mut session = DesignSession::new().expect("session");
        let page = session.active_page().expect("page");
        let greeting = session
            .create_node(page, NodeKind::Text("Hi {{name}}".into()))
            .expect("greeting");
        let photo = session
            .create_node(page, NodeKind::Image(png(4, 4, [0, 0, 255, 255])))
            .expect("photo");
        session.set_name(photo, "photo").expect("name");
        session
            .set_placeholder(photo, Some(Placeholder::default()))
            .expect("placeholder");
        session.add_undo_step("Template").expect("commit");
        (session, greeting, photo)
    }

    #[test]
    fn the_default_fetcher_decodes_data_uris_and_nothing_else() {
        let image = png(1, 1, [0, 0, 0, 255]);
        assert_eq!(
            DataUriFetcher.fetch(&data_uri(&image)).expect("decode"),
            image
        );
        assert!(DataUriFetcher.fetch("https://example.com/a.png").is_err());
        assert!(DataUriFetcher.fetch("file:///etc/hosts").is_err());
    }

    #[test]
    fn a_directory_fetcher_reads_only_inside_its_root() {
        let directory = tempfile::tempdir().expect("directory");
        let root = directory.path().join("images");
        std::fs::create_dir(&root).expect("root");
        std::fs::write(root.join("photo.png"), b"inside").expect("inside");
        std::fs::write(directory.path().join("secret.png"), b"outside").expect("outside");
        let fetcher = DirectoryFetcher::new(&root).expect("fetcher");
        assert_eq!(fetcher.fetch("photo.png").expect("relative"), b"inside");
        let absolute = root.join("photo.png");
        assert_eq!(
            fetcher
                .fetch(&format!("file://{}", absolute.display()))
                .expect("file URI inside the root"),
            b"inside"
        );
        assert!(fetcher.fetch("../secret.png").is_err());
        let outside = directory.path().join("secret.png");
        assert!(fetcher.fetch(&outside.display().to_string()).is_err());
        assert!(fetcher.fetch("missing.png").is_err());
    }

    #[test]
    fn an_http_fetcher_wants_https_unless_told_and_keeps_to_its_allowlist() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        let address = listener.local_addr().expect("address");
        let image = png(2, 2, [0, 255, 0, 255]);
        let server = std::thread::spawn({
            let image = image.clone();
            move || {
                let (mut stream, _) = listener.accept().expect("connection");
                let mut request = Vec::new();
                let mut buffer = [0; 1024];
                while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                    let read = stream.read(&mut buffer).expect("read the request");
                    if read == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..read]);
                }
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    image.len()
                )
                .expect("write the head");
                stream.write_all(&image).expect("write the body");
            }
        });
        let url = format!("http://{address}/photo.png");
        assert!(
            HttpFetcher::new().fetch(&url).is_err(),
            "plain http is opt in"
        );
        assert!(
            HttpFetcher::new()
                .allow_plain_http()
                .allow_hosts(["images.example.com"])
                .fetch(&url)
                .is_err(),
            "the allowlist refuses before connecting"
        );
        let fetched = HttpFetcher::new()
            .allow_plain_http()
            .allow_hosts(["127.0.0.1"])
            .fetch(&url)
            .expect("fetch");
        assert_eq!(fetched, image);
        server.join().expect("server");
    }

    #[test]
    fn filling_a_template_sets_its_text_and_placeholder_images() {
        let (mut session, greeting, photo) = template();
        let before = bitmap_asset(&session, photo);
        let red = png(4, 4, [255, 0, 0, 255]);
        session
            .fill_template(&row(&[("name", "Ada"), ("photo", &data_uri(&red))]))
            .expect("fill");
        assert_eq!(shown_text(&session, greeting), "Hi Ada");
        assert_ne!(bitmap_asset(&session, photo), before);
        assert!(session.add_undo_step("Fill").expect("commit"));
        assert!(session.undo().expect("undo"));
        assert_eq!(shown_text(&session, greeting), "Hi {{name}}");
        assert_eq!(bitmap_asset(&session, photo), before);
    }

    #[test]
    fn a_row_that_fails_changes_nothing() {
        let (mut session, greeting, photo) = template();
        session
            .edit_with_design_op(&DesignOp::CreateVariableCollection {
                name: "Numbers".into(),
                modes: vec!["Default".into()],
            })
            .expect("collection");
        session
            .edit_with_design_op(&DesignOp::CreateVariable {
                collection: "Numbers".into(),
                name: "zz_count".into(),
                kind: DesignVariableType::Float,
                value: serde_json::json!(3),
                options: Vec::new(),
            })
            .expect("number");
        session.add_undo_step("Numbers").expect("commit");
        session.set_name(photo, "b_photo").expect("rename");
        let assets = session.document().raw_assets().len();
        let before = bitmap_asset(&session, photo);
        let green = png(4, 4, [0, 255, 0, 255]);
        let error = session
            .fill_template(&row(&[
                ("a_title", "Hello"),
                ("b_photo", &data_uri(&green)),
                ("zz_count", "not a number"),
            ]))
            .expect_err("a number variable is not text");
        assert!(error.to_string().contains("zz_count"), "{error}");
        assert_eq!(session.text_variable("a_title"), None);
        assert_eq!(bitmap_asset(&session, photo), before);
        assert_eq!(session.document().raw_assets().len(), assets);
        assert_eq!(shown_text(&session, greeting), "Hi {{name}}");

        assert!(
            session
                .fill_template(&row(&[("b_photo", "https://example.com/a.png")]))
                .is_err(),
            "the default fetcher fetches no URLs"
        );
        assert_eq!(bitmap_asset(&session, photo), before);
    }

    #[test]
    fn batch_export_fills_a_copy_for_each_row() {
        let (session, greeting, _) = template();
        let page = session.active_page().expect("page");
        let undo_depth = session.doc().history.undo_depth();
        let assets = session.document().raw_assets().len();
        let results = session.batch_export(
            &[
                row(&[("name", "Ada")]),
                row(&[("name", "Grace Hopper")]),
                row(&[("", "no key")]),
            ],
            page,
            ExportEncoding::Png,
            ExportSizing::Scale(1.0),
        );
        assert_eq!(results.len(), 3);
        let first = results[0].as_ref().expect("first row");
        let second = results[1].as_ref().expect("second row");
        assert_ne!(first.bytes, second.bytes);
        assert!(results[2].is_err(), "an empty key fails only its own row");

        assert!(session.text_variables().is_empty());
        assert_eq!(shown_text(&session, greeting), "Hi {{name}}");
        assert_eq!(session.doc().history.undo_depth(), undo_depth);
        assert_eq!(session.document().raw_assets().len(), assets);
    }

    #[test]
    fn a_viewer_cannot_fill_a_template() {
        let (mut session, _, _) = template();
        session.set_role(Role::Viewer);
        assert!(matches!(
            session.fill_template(&row(&[("name", "Ada")])),
            Err(ApiError::ViewOnly)
        ));
    }
}
