//! Error type for `.fig` interop.
//!
//! One `thiserror` enum spanning the whole pipeline — codec, container, and
//! mapping — because a caller importing a `.fig` does not care *which* layer
//! failed, only *that* it did and roughly why. The variants are deliberately
//! coarse: byte-level codec failures all collapse to [`FigError::Truncated`]
//! (the only way a structurally-valid Kiwi stream fails is by running out of
//! bytes mid-read), while semantic problems carry a message string so the log
//! is actionable without a debugger.

/// Everything that can go wrong reading a `.fig` file.
#[derive(Debug, thiserror::Error)]
pub enum FigError {
    /// A read ran past the end of the buffer. In Kiwi this is the *only*
    /// low-level failure mode: the format has no length-prefixed framing for
    /// scalars, so a corrupt or partial stream manifests as "not enough bytes".
    #[error("unexpected end of input (truncated or corrupt stream)")]
    Truncated,

    /// The 8-byte container prelude did not match the expected magic.
    #[error("bad .fig header: {0}")]
    BadHeader(String),

    /// The container version is outside the range this reader understands.
    #[error("unsupported .fig version: {0}")]
    UnsupportedVersion(u32),

    /// A zlib/DEFLATE block failed to inflate.
    #[error("failed to inflate a .fig block: {0}")]
    Inflate(String),

    /// The embedded Kiwi schema was malformed (bad def kind, dangling type
    /// reference, etc.).
    #[error("invalid Kiwi schema: {0}")]
    Schema(String),

    /// A field referenced a type id that is neither a builtin nor a valid
    /// definition index in the schema.
    #[error("unknown Kiwi type id: {0}")]
    UnknownType(i32),

    /// The decoded value tree could not be mapped onto the Fantaisa doc model.
    #[error("mapping .fig into the doc model failed: {0}")]
    Mapping(String),
}

/// Result alias used throughout the crate.
pub type FigResult<T> = Result<T, FigError>;
