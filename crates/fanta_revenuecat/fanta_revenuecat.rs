use serde::Deserialize;
use thiserror::Error;

#[derive(Debug, Error, Clone)]
pub enum RevenueCatError {
    #[error("RevenueCat is only available on macOS")]
    UnsupportedPlatform,
    #[error("RevenueCat bridge is unavailable: {0}")]
    BridgeUnavailable(String),
    #[error("invalid RevenueCat input: {0}")]
    InvalidInput(String),
    #[error("RevenueCat must be configured on the app's main thread")]
    WrongThread,
    #[error("RevenueCat was already configured with a different public API key")]
    DifferentApiKey,
    #[error("RevenueCat was already configured for a different user; call log_in")]
    DifferentAppUser,
    #[error("purchase was cancelled")]
    Cancelled,
    #[error("RevenueCat request failed: {0}")]
    Sdk(String),
    #[error("invalid RevenueCat response: {0}")]
    InvalidResponse(String),
}

#[derive(Debug, Clone, Deserialize)]
pub struct Product {
    pub identifier: String,
    pub title: String,
    pub description: String,
    pub localized_price: String,
    pub currency_code: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Package {
    pub identifier: String,
    pub product: Product,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Offering {
    pub identifier: String,
    pub packages: Vec<Package>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Offerings {
    pub current: Option<Offering>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CustomerInfo {
    pub app_user_id: String,
    pub active_entitlements: Vec<String>,
    pub active_subscriptions: Vec<String>,
    pub purchased_product_identifiers: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Purchase {
    pub product_id: String,
    pub transaction_id: Option<String>,
    pub customer_info: CustomerInfo,
}

#[derive(Clone, Copy)]
pub struct RevenueCat;

impl RevenueCat {
    /// Configure once on the app's main thread with the public Apple SDK key and backend user UUID.
    pub fn configure(public_api_key: &str, app_user_id: &str) -> Result<Self, RevenueCatError> {
        if public_api_key.is_empty() || app_user_id.is_empty() {
            return Err(RevenueCatError::InvalidInput(
                "public API key and app user ID are required".into(),
            ));
        }
        platform::configure(public_api_key, app_user_id)?;
        Ok(Self)
    }

    /// Configure once, or identify a new backend user in an existing SDK session.
    pub async fn get_or_configure(
        public_api_key: &str,
        app_user_id: &str,
    ) -> Result<Self, RevenueCatError> {
        match Self::configure(public_api_key, app_user_id) {
            Ok(client) => Ok(client),
            Err(RevenueCatError::DifferentAppUser) => {
                let client = Self;
                client.log_in(app_user_id).await?;
                Ok(client)
            }
            Err(error) => Err(error),
        }
    }

    pub async fn log_in(&self, user_uuid: &str) -> Result<CustomerInfo, RevenueCatError> {
        platform::request("log_in", Some(user_uuid)).await
    }

    pub async fn log_out(&self) -> Result<CustomerInfo, RevenueCatError> {
        platform::request("log_out", None).await
    }

    pub async fn offerings(&self) -> Result<Offerings, RevenueCatError> {
        platform::request("offerings", None).await
    }

    pub async fn products(&self, product_ids: &[&str]) -> Result<Vec<Product>, RevenueCatError> {
        if product_ids.is_empty() {
            return Ok(Vec::new());
        }
        let identifiers = serde_json::to_string(product_ids)
            .map_err(|error| RevenueCatError::InvalidInput(error.to_string()))?;
        platform::request("products", Some(&identifiers)).await
    }

    pub async fn purchase(&self, product_id: &str) -> Result<Purchase, RevenueCatError> {
        platform::request("purchase", Some(product_id)).await
    }

    pub async fn restore(&self) -> Result<CustomerInfo, RevenueCatError> {
        platform::request("restore", None).await
    }

    pub async fn customer_info(&self) -> Result<CustomerInfo, RevenueCatError> {
        platform::request("customer_info", None).await
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use super::RevenueCatError;
    use serde::de::DeserializeOwned;

    pub fn configure(_public_api_key: &str, _app_user_id: &str) -> Result<(), RevenueCatError> {
        Err(RevenueCatError::UnsupportedPlatform)
    }

    pub async fn request<T: DeserializeOwned>(
        _operation: &str,
        _argument: Option<&str>,
    ) -> Result<T, RevenueCatError> {
        Err(RevenueCatError::UnsupportedPlatform)
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::RevenueCatError;
    use futures::channel::oneshot;
    use libloading::Library;
    use serde::de::DeserializeOwned;
    use std::{
        env,
        ffi::{CStr, CString, c_char, c_int, c_void},
        path::PathBuf,
        sync::OnceLock,
    };

    type ConfigureFn = unsafe extern "C" fn(*const c_char, *const c_char) -> c_int;
    type CompletionFn = extern "C" fn(*mut c_void, c_int, *const c_char);
    type RequestFn = unsafe extern "C" fn(*const c_char, *const c_char, *mut c_void, CompletionFn);

    struct Native {
        configure: ConfigureFn,
        request: RequestFn,
    }

    static NATIVE: OnceLock<Result<Native, RevenueCatError>> = OnceLock::new();

    fn native() -> Result<&'static Native, RevenueCatError> {
        NATIVE
            .get_or_init(|| {
                let path = bridge_path()?;
                // The library must remain loaded while asynchronous RevenueCat callbacks are pending.
                let library = unsafe { Library::new(&path) }
                    .map_err(|error| RevenueCatError::BridgeUnavailable(error.to_string()))?;
                let configure = unsafe {
                    *library
                        .get::<ConfigureFn>(b"fanta_rc_configure")
                        .map_err(|error| RevenueCatError::BridgeUnavailable(error.to_string()))?
                };
                let request = unsafe {
                    *library
                        .get::<RequestFn>(b"fanta_rc_request")
                        .map_err(|error| RevenueCatError::BridgeUnavailable(error.to_string()))?
                };
                std::mem::forget(library);
                Ok(Native { configure, request })
            })
            .as_ref()
            .map_err(Clone::clone)
    }

    fn bridge_path() -> Result<PathBuf, RevenueCatError> {
        if let Some(path) = env::var_os("FANTA_REVENUECAT_BRIDGE_PATH") {
            return Ok(PathBuf::from(path));
        }
        let executable = env::current_exe()
            .map_err(|error| RevenueCatError::BridgeUnavailable(error.to_string()))?;
        let executable_directory = executable.parent().ok_or_else(|| {
            RevenueCatError::BridgeUnavailable("app executable has no parent directory".into())
        })?;
        Ok(executable_directory.join("../Frameworks/libFantaRevenueCatBridge.dylib"))
    }

    pub fn configure(public_api_key: &str, app_user_id: &str) -> Result<(), RevenueCatError> {
        let public_api_key = CString::new(public_api_key)
            .map_err(|error| RevenueCatError::InvalidInput(error.to_string()))?;
        let app_user_id = CString::new(app_user_id)
            .map_err(|error| RevenueCatError::InvalidInput(error.to_string()))?;
        let result =
            unsafe { (native()?.configure)(public_api_key.as_ptr(), app_user_id.as_ptr()) };
        match result {
            0 => Ok(()),
            1 => Err(RevenueCatError::WrongThread),
            2 => Err(RevenueCatError::DifferentApiKey),
            4 => Err(RevenueCatError::DifferentAppUser),
            code => Err(RevenueCatError::Sdk(format!(
                "configuration failed with code {code}"
            ))),
        }
    }

    pub async fn request<T: DeserializeOwned>(
        operation: &str,
        argument: Option<&str>,
    ) -> Result<T, RevenueCatError> {
        let operation = CString::new(operation)
            .map_err(|error| RevenueCatError::InvalidInput(error.to_string()))?;
        let argument = argument
            .map(CString::new)
            .transpose()
            .map_err(|error| RevenueCatError::InvalidInput(error.to_string()))?;
        let (sender, receiver) = oneshot::channel::<(i32, String)>();
        let native = native()?;
        let context = Box::into_raw(Box::new(sender)).cast::<c_void>();
        unsafe {
            (native.request)(
                operation.as_ptr(),
                argument
                    .as_ref()
                    .map_or(std::ptr::null(), |value| value.as_ptr()),
                context,
                complete,
            );
        }
        let (status, payload) = receiver
            .await
            .map_err(|_| RevenueCatError::Sdk("RevenueCat callback was dropped".into()))?;
        match status {
            0 => serde_json::from_str(&payload)
                .map_err(|error| RevenueCatError::InvalidResponse(error.to_string())),
            1 => Err(RevenueCatError::Cancelled),
            _ => Err(RevenueCatError::Sdk(payload)),
        }
    }

    extern "C" fn complete(context: *mut c_void, status: c_int, payload: *const c_char) {
        if context.is_null() {
            return;
        }
        let sender = unsafe { Box::from_raw(context.cast::<oneshot::Sender<(i32, String)>>()) };
        let response = if payload.is_null() {
            String::new()
        } else {
            unsafe { CStr::from_ptr(payload) }
                .to_string_lossy()
                .into_owned()
        };
        if sender.send((status, response)).is_err() {
            log::debug!("RevenueCat response arrived after its caller was dropped");
        }
    }
}
