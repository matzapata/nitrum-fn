use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, ItemFn};

/// Marks the function entrypoint. Expands to `run(service_fn(...))` plus wasm exports.
///
/// ```ignore
/// #[runtime::main]
/// fn handler(_req: Request) -> Result<Value, Error> {
///     Ok(json!({ "message": "Hello, world!" }))
/// }
/// ```
#[proc_macro_attribute]
pub fn main(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemFn);
    let name = &input.sig.ident;

    if input.sig.asyncness.is_some() {
        return syn::Error::new_spanned(
            input.sig.fn_token,
            "#[runtime::main] async handlers are not supported yet",
        )
        .to_compile_error()
        .into();
    }

    TokenStream::from(quote! {
        #input

        fn main() {
            ::runtime::run(::runtime::service_fn(#name));
        }

        #[cfg(target_arch = "wasm32")]
        pub use ::runtime::invoke;

        #[cfg(target_arch = "wasm32")]
        #[no_mangle]
        pub extern "C" fn nitrum_start() {
            main();
        }
    })
}
