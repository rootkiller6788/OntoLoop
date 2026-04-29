pub mod compatibility_loader;
pub mod facade_guard;
#[path = "../gitmemory_core/mod.rs"]
pub mod gitmemory_core;
pub mod host;
pub mod lifecycle;
pub mod markdown_preprocess;
pub mod registry;
pub mod resolver;
pub mod signature;

pub use compatibility_loader::{CompatibilityDiscoveryReport, PluginCompatibilityLoader};
pub use host::{
    ExternalPluginLoadRequest, ExternalPluginLoader, PluginHost, SubprocessPluginLoader,
};
pub use lifecycle::{PluginLifecycleManager, PluginRuntimeRecord};
pub use markdown_preprocess::{MarkdownPreprocessPlugin, MarkdownPreprocessRequest, MarkdownPreprocessResult};
pub use registry::{PluginRegistration, PluginRegistry, PluginSourceKind};
pub use resolver::{PluginResolveRequest, PluginResolver, ResolvedPlugin, negotiate_with_manifest};
pub use signature::{
    PluginSignatureMaterial, compute_plugin_signature, signature_material, verify_install_request,
};
