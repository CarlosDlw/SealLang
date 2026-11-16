//! Derive macros for Seal's compiler infrastructure.
//!
//! Currently this crate provides `#[derive(StableHash)]`. Procedural macros are
//! kept in their own crate so that the rest of the compiler does not pay the
//! `syn`/`quote` build cost.

#![warn(missing_docs)]

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{Data, DeriveInput, Fields, GenericParam, parse_macro_input, parse_quote};

/// Derives [`seal_hash::StableHash`] for a struct or enum.
///
/// Fields are hashed in declaration order. Enums also hash the variant index, so
/// two variants with identical field types never collide.
///
/// The derive adds `T: ::seal_hash::StableHash` bounds to every type parameter.
///
/// # Examples
///
/// ```
/// use seal_macros::StableHash;
///
/// #[derive(StableHash)]
/// struct Span {
///     start: u32,
///     end: u32,
/// }
///
/// #[derive(StableHash)]
/// enum Level {
///     Error,
///     Warning { count: u32 },
/// }
/// ```
#[proc_macro_derive(StableHash)]
pub fn derive_stable_hash(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match expand_stable_hash(input) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

fn expand_stable_hash(mut input: DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let name = input.ident.clone();

    // Every type parameter must be hashable for the derived implementation to
    // compile. Adding the bound here produces a clear error at the use site.
    for param in &mut input.generics.params {
        if let GenericParam::Type(type_param) = param {
            type_param
                .bounds
                .push(parse_quote!(::seal_hash::StableHash));
        }
    }
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let body = match &input.data {
        Data::Struct(data) => {
            let pattern = variant_pattern(None, &data.fields);
            let statements = field_statements(&data.fields);
            quote! {
                let #pattern = self;
                #(#statements)*
            }
        }
        Data::Enum(data) => {
            let mut arms = Vec::with_capacity(data.variants.len());
            for (index, variant) in data.variants.iter().enumerate() {
                let variant_index = u32::try_from(index).map_err(|_| {
                    syn::Error::new_spanned(
                        &variant.ident,
                        "enums with more than u32::MAX variants are not supported",
                    )
                })?;
                let pattern = variant_pattern(Some(&variant.ident), &variant.fields);
                let statements = field_statements(&variant.fields);
                arms.push(quote! {
                    #pattern => {
                        hasher.write_u32(#variant_index);
                        #(#statements)*
                    }
                });
            }
            // An empty enum can never be constructed, so an empty match is
            // exhaustive and no field hashing is needed.
            quote! {
                match self {
                    #(#arms)*
                }
            }
        }
        Data::Union(data) => {
            return Err(syn::Error::new_spanned(
                data.union_token,
                "`StableHash` cannot be derived for unions: their field order is not stable",
            ));
        }
    };

    Ok(quote! {
        #[automatically_derived]
        impl #impl_generics ::seal_hash::StableHash for #name #ty_generics #where_clause {
            fn stable_hash(&self, hasher: &mut ::seal_hash::StableHasher) {
                #body
            }
        }
    })
}

/// Produces the destructuring pattern for a struct or enum variant.
///
/// Fields are bound to `field0`, `field1`, ... in declaration order, including
/// named fields (which are renamed in the pattern), so the generated body never
/// depends on user-chosen identifiers.
fn variant_pattern(variant: Option<&syn::Ident>, fields: &Fields) -> proc_macro2::TokenStream {
    let path = match variant {
        Some(ident) => quote!(Self::#ident),
        None => quote!(Self),
    };
    match fields {
        Fields::Named(named) => {
            let entries = named.named.iter().enumerate().map(|(index, field)| {
                let field_name = field
                    .ident
                    .as_ref()
                    .expect("named fields always have an identifier");
                let binding = format_ident!("field{index}");
                quote!(#field_name: #binding)
            });
            quote! { #path { #(#entries),* } }
        }
        Fields::Unnamed(unnamed) => {
            let bindings = (0..unnamed.unnamed.len()).map(|index| format_ident!("field{index}"));
            quote! { #path ( #(#bindings),* ) }
        }
        Fields::Unit => quote!(#path),
    }
}

/// Produces one hashing statement per field, in declaration order.
fn field_statements(fields: &Fields) -> Vec<proc_macro2::TokenStream> {
    (0..fields.len())
        .map(|index| {
            let binding = format_ident!("field{index}");
            quote! {
                ::seal_hash::StableHash::stable_hash(#binding, hasher);
            }
        })
        .collect()
}
