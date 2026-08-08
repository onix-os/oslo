use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{parse_macro_input, DeriveInput};

pub trait DeriveGenerator: EnumGenerator + StructGenerator {
    fn complete(input: &syn::DeriveInput, tokens: TokenStream) -> TokenStream;

    fn derive(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
        let input = parse_macro_input!(input as DeriveInput);

        let expanded = match &input.data {
            syn::Data::Enum(data) => <Self as EnumGenerator>::generate(&input.ident, data),
            syn::Data::Struct(data) => <Self as StructGenerator>::generate(&input.ident, data),
            _ => unimplemented!(),
        };

        Self::complete(&input, expanded).into()
    }
}

pub trait EnumGenerator {
    fn generate(input: &syn::Ident, enumm: &syn::DataEnum) -> TokenStream;
}

pub trait StructGenerator {
    fn generate(input: &syn::Ident, strukt: &syn::DataStruct) -> TokenStream;
}

pub trait MatchEnumGenerator {
    const DEREF: bool = false;
    const SELF: &'static str = "self";

    fn complete(input: TokenStream) -> TokenStream {
        input
    }

    fn case_named(
        _input: &syn::Ident,
        _variant: &syn::Ident,
        _fields: &syn::FieldsNamed,
    ) -> TokenStream {
        quote! {}
    }

    fn case_unnamed(
        _input: &syn::Ident,
        _variant: &syn::Ident,
        _fields: &syn::FieldsUnnamed,
    ) -> TokenStream {
        quote! {}
    }

    fn case_unit(_input: &syn::Ident, _variant: &syn::Ident) -> TokenStream {
        quote! {}
    }
}

impl<T: MatchEnumGenerator> EnumGenerator for T {
    fn generate(input_ident: &syn::Ident, enumm: &syn::DataEnum) -> TokenStream {
        let mut cases = Vec::with_capacity(enumm.variants.len());

        for variant in &enumm.variants {
            let variant_ident = &variant.ident;

            match &variant.fields {
                syn::Fields::Named(fields) => {
                    cases.push(T::case_named(input_ident, variant_ident, fields));
                }

                syn::Fields::Unnamed(fields) => {
                    cases.push(T::case_unnamed(input_ident, variant_ident, fields));
                }

                syn::Fields::Unit => cases.push(T::case_unit(input_ident, variant_ident)),
            }
        }

        let self_ident = format_ident!("{}", T::SELF);
        let deref = if T::DEREF { Some(quote! { * }) } else { None };

        T::complete(quote! {
            match #deref#self_ident {
                #(#cases)*
            }
        })
    }
}

pub trait Hint
where
    Self: Sized,
{
    fn key_value(_key: String, _value: String) -> Option<Self> {
        None
    }

    fn unit(_name: String) -> Option<Self> {
        None
    }
}

/// The first `#[name(...)]` hint on `attrs`, read as either `#[name(word)]` or
/// `#[name(key = "value")]`.
///
/// Rewritten for syn 2, which removed `Attribute::parse_meta` and `NestedMeta` in favour of
/// `parse_nested_meta`. That is a callback rather than a list, so "the first entry wins" — which
/// is what the old loop meant by returning out of it — is expressed by keeping the first hit and
/// ignoring the rest.
pub fn search_hint<T: Hint>(name: &str, attrs: &[syn::Attribute]) -> Option<T> {
    let mut found = None;

    for attr in attrs {
        if !attr.path().is_ident(name) {
            continue;
        }
        // A malformed hint is skipped rather than fatal, as it was before: these attributes are
        // also read by other derives on the same type, and one of them not understanding an entry
        // is not an error in this one.
        let _ = attr.parse_nested_meta(|meta| {
            if found.is_some() {
                return Ok(());
            }
            let Some(ident) = meta.path.get_ident() else {
                return Ok(());
            };
            let key = ident.to_string();

            // `key = "value"` if a value follows, and a bare word otherwise. syn 2 signals which
            // by whether the input is at an `=`, so the shape is decided here rather than by the
            // two enum variants the old code matched on.
            found = match meta.value() {
                Ok(value) => {
                    let literal: syn::LitStr = value.parse()?;
                    T::key_value(key, literal.value())
                }
                Err(_) => T::unit(key),
            };
            Ok(())
        });

        if found.is_some() {
            break;
        }
    }

    found
}
