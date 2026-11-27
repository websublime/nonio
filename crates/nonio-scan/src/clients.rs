//! HTTP client libraries, as configuration (the way D17 treats route frameworks): how each one makes an instance
//! with a base URL, and how a request is sent with it. Calls are matched by resolving the callee to the library's
//! import or to an instance's binding ([`crate::walk`]), never by the name written at the call (D16).

/// One client library.
pub struct ClientLib {
    /// The npm package, as `vendor:<pkg>` names it.
    pub pkg: &'static str,
    /// Methods that make an instance: `axios.create(opts)`.
    pub factories: &'static [&'static str],
    /// The option of the factory that holds the base URL.
    pub prefix_key: &'static str,
    /// Whether the library and its instances are also called directly with a URL: `ky('/x')`, `api('/x')`.
    pub callable: bool,
}

pub const CLIENT_LIBS: [ClientLib; 4] = [
    ClientLib { pkg: "axios", factories: &["create"], prefix_key: "baseURL", callable: false },
    ClientLib { pkg: "ky", factories: &["create", "extend"], prefix_key: "prefixUrl", callable: true },
    ClientLib { pkg: "got", factories: &["extend"], prefix_key: "prefixUrl", callable: true },
    ClientLib { pkg: "ofetch", factories: &["create"], prefix_key: "baseURL", callable: true },
];

/// Methods that send a request, on a library or an instance.
pub const REQUEST_METHODS: [&str; 7] = ["get", "post", "put", "patch", "delete", "head", "options"];

/// The library behind `vendor:<pkg>` (or a subpath of it).
pub fn client_lib(target: &str) -> Option<&'static ClientLib> {
    let pkg = target.strip_prefix("vendor:")?;
    let pkg = pkg.split('/').next()?;
    CLIENT_LIBS.iter().find(|l| l.pkg == pkg)
}
