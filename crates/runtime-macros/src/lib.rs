use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, ItemFn};

/// Marks the function entrypoint. Expands to a wasm `invoke` that lazy-registers the handler.
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
            "#[runtime::main] requires a synchronous fn",
        )
        .to_compile_error()
        .into();
    }

    TokenStream::from(quote! {
        #input

        #[cfg(target_arch = "wasm32")]
        #[no_mangle]
        pub extern "C" fn invoke(ptr: i32, len: i32) -> i32 {
            static INIT: ::std::sync::Once = ::std::sync::Once::new();
            INIT.call_once(|| {
                ::runtime::run(::runtime::service_fn(#name));
            });
            ::runtime::__invoke(ptr, len)
        }
    })
}
