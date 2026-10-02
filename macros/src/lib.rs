use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, parse::{Parse, ParseStream, Result}, Ident, Expr, token::Comma};

struct CustomSyntax {
    key: Ident,
    val: Expr,
}

impl Parse for CustomSyntax {
    fn parse(input: ParseStream) -> Result<Self> {
        let key: Ident = input.parse()?;
        let _comma: Comma = input.parse()?;
        let val: Expr = input.parse()?;

        Ok(CustomSyntax {
            key,
            val,
        })
    }
}

#[proc_macro]
pub fn elf(token: TokenStream) -> TokenStream {
    let token_ = parse_macro_input!(token as CustomSyntax);
    let k = token_.key;
    let v = token_.val;

    let expanded = quote! {
        #[no_mangle]
        pub static #k: [u8; #v.len()] = {
            let v_ = #v.as_bytes();
            let mut elf  = [0_u8; #v.len()];

            let mut i = 0;
            while i < v_.len() {
                elf[i] = v_[i];
                i += 1;
            }

            elf
        };
    };

    TokenStream::from(expanded)
}