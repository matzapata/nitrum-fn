use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, ItemFn};

/// Marks the function entrypoint. Expands to a wasm `invoke` that lazy-registers the handler.
///
/// Sync handlers:
/// ```ignore
/// #[runtime::main]
/// fn handler(_req: Request) -> Result<Value, Error> { ... }
/// ```
///
/// Async handlers (outbound `.send().await`):
/// ```ignore
/// #[runtime::main]
/// async fn handler(_req: Request) -> Result<Value, Error> { ... }
/// ```
#[proc_macro_attribute]
pub fn main(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemFn);
    let name = &input.sig.ident;
    let is_async = input.sig.asyncness.is_some();

    let register = if is_async {
        quote! {
            ::runtime::run(::runtime::service_fn(|req| {
                ::runtime::block_on(#name(req))
            }));
        }
    } else {
        quote! {
            ::runtime::run(::runtime::service_fn(#name));
        }
    };

    TokenStream::from(quote! {
        #input

        #[cfg(target_arch = "wasm32")]
        #[no_mangle]
        pub extern "C" fn invoke(ptr: i32, len: i32) -> i32 {
            static INIT: ::std::sync::Once = ::std::sync::Once::new();
            INIT.call_once(|| {
                #register
            });
            ::runtime::__invoke(ptr, len)
        }
    })
}
