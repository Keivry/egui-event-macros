// SPDX-License-Identifier: MIT OR Apache-2.0

use {
    proc_macro::TokenStream,
    quote::{format_ident, quote},
    syn::{Data, DeriveInput, Fields, GenericParam, parse_macro_input},
};

/// Derive the `egui_event::Event` marker trait for a struct or enum.
///
/// # Generics
///
/// Type parameters are supported. For each type parameter `T`, the macro adds
/// `T: Send + Sync + 'static` bounds so the impl satisfies all trait
/// requirements automatically.
///
/// Lifetime parameters are **not** supported and produce a compile-time error.
///
/// # Examples
///
/// ```rust,ignore
/// use egui_event_macros::Event;
///
/// #[derive(Event)]
/// struct Login {
///     username: String,
/// }
///
/// #[derive(Event)]
/// enum UiAction {
///     ButtonClicked(String),
///     SliderMoved(f32),
/// }
///
/// // Generic structs are supported:
/// #[derive(Event)]
/// struct Wrapper<T: Send + Sync + 'static> {
///     inner: T,
/// }
/// ```
#[proc_macro_derive(Event)]
pub fn derive_event(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;

    // Reject lifetime parameters.
    for param in &input.generics.params {
        if let GenericParam::Lifetime(lt) = param {
            return syn::Error::new_spanned(
                lt,
                "#[derive(Event)] does not support lifetime parameters",
            )
            .to_compile_error()
            .into();
        }
    }

    // Reject unions.
    if matches!(&input.data, Data::Union(_)) {
        return syn::Error::new_spanned(
            name,
            "#[derive(Event)] is only supported on structs and enums, not unions",
        )
        .to_compile_error()
        .into();
    }

    // Clone generics and add Send + Sync + 'static bounds to type params.
    let mut augmented_generics = input.generics.clone();
    for param in &mut augmented_generics.params {
        if let GenericParam::Type(tp) = param {
            tp.bounds.push(syn::parse_quote!(Send));
            tp.bounds.push(syn::parse_quote!(Sync));
            tp.bounds.push(syn::parse_quote!('static));
        }
    }

    let (impl_generics, ty_generics, where_clause) = augmented_generics.split_for_impl();

    let expanded = quote! {
        impl #impl_generics ::egui_event::Event for #name #ty_generics #where_clause {}
    };

    expanded.into()
}

/// Derive per-variant event types from an enum.
///
/// For each enum variant, a dedicated struct is generated whose name is the
/// concatenation of the enum name and the variant name. The generated struct
/// name is simply the **variant name** (`{Variant}`), and `egui_event::Event`
/// is implemented for it. The enum itself also gets an
/// `Event` impl, and a `From<VariantStruct> for Enum` conversion is generated
/// for every variant.
///
/// # Limitations
///
/// - Only supported on enums.
/// - Generic parameters (type and lifetime) are not supported.
/// - Field and variant **attributes** (e.g. `#[serde(rename)]`, `#[doc = "..."]`) are **not**
///   propagated to the generated structs. Annotate the generated types directly if you need
///   attributes on them.
///
/// # Name Conflicts
///
/// This macro generates a struct for each variant with **the same name as the
/// variant itself**. If a type with that name already exists in the same module
/// scope the compiler will emit an error. Rename the conflicting type or move
/// the generated types into a dedicated submodule.
/// # Field Visibility
///
/// The visibility of the generated struct fields matches the visibility of the
/// enum itself. A `pub enum` generates `pub` fields; a `pub(crate) enum`
/// generates `pub(crate)` fields; a private enum generates private fields.
///
/// # Example
///
/// ```rust,ignore
/// use egui_event_macros::EventSet;
///
/// #[derive(EventSet)]
/// pub enum UiAction {
///     Login { username: String },
///     Logout,
///     ButtonClicked(String),
/// }
///
/// // Generated:
/// //   pub struct Login { pub username: String }
/// //   pub struct Logout;
/// //   pub struct ButtonClicked(pub String);
/// //   impl Event for Login {}
/// //   impl Event for Logout {}
/// //   impl Event for ButtonClicked {}
/// //   impl Event for UiAction {}
/// //   impl From<Login>        for UiAction { ... }
/// //   impl From<Logout>       for UiAction { ... }
/// //   impl From<ButtonClicked> for UiAction { ... }
/// ```
#[proc_macro_derive(EventSet)]
pub fn derive_event_set(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let enum_name = &input.ident;
    let vis = &input.vis;

    // Only enums are supported.
    let data_enum = match &input.data {
        Data::Enum(data_enum) => data_enum,
        _ => {
            return syn::Error::new_spanned(
                enum_name,
                "#[derive(EventSet)] is only supported on enums",
            )
            .to_compile_error()
            .into();
        }
    };

    // Reject any generic parameters (lifetime or type).
    if let Some(param) = (&input.generics.params).into_iter().next() {
        match param {
            GenericParam::Lifetime(lt) => {
                return syn::Error::new_spanned(
                    lt,
                    "#[derive(EventSet)] does not support lifetime parameters",
                )
                .to_compile_error()
                .into();
            }
            GenericParam::Type(tp) => {
                return syn::Error::new_spanned(
                    tp,
                    "#[derive(EventSet)] does not support generic enums",
                )
                .to_compile_error()
                .into();
            }
            GenericParam::Const(cp) => {
                return syn::Error::new_spanned(
                    cp,
                    "#[derive(EventSet)] does not support const generics",
                )
                .to_compile_error()
                .into();
            }
        }
    }

    let mut struct_defs = Vec::new();
    let mut event_impls = Vec::new();
    let mut from_impls = Vec::new();

    for variant in &data_enum.variants {
        let variant_name = &variant.ident;
        let struct_name = format_ident!("{}", variant_name);

        match &variant.fields {
            Fields::Unit => {
                struct_defs.push(quote! {
                    #vis struct #struct_name;
                });
                from_impls.push(quote! {
                    impl ::core::convert::From<#struct_name> for #enum_name {
                        fn from(_: #struct_name) -> Self {
                            #enum_name::#variant_name
                        }
                    }
                });
            }
            Fields::Named(fields_named) => {
                let field_defs: Vec<_> = fields_named
                    .named
                    .iter()
                    .map(|f| {
                        let fname = &f.ident;
                        let fty = &f.ty;
                        quote! { #vis #fname: #fty }
                    })
                    .collect();
                let field_names: Vec<_> = fields_named
                    .named
                    .iter()
                    .map(|f| f.ident.as_ref().unwrap())
                    .collect();
                struct_defs.push(quote! {
                    #vis struct #struct_name {
                        #(#field_defs),*
                    }
                });
                from_impls.push(quote! {
                    impl ::core::convert::From<#struct_name> for #enum_name {
                        fn from(v: #struct_name) -> Self {
                            #enum_name::#variant_name {
                                #(#field_names: v.#field_names),*
                            }
                        }
                    }
                });
            }
            Fields::Unnamed(fields_unnamed) => {
                let field_defs: Vec<_> = fields_unnamed
                    .unnamed
                    .iter()
                    .map(|f| {
                        let fty = &f.ty;
                        quote! { #vis #fty }
                    })
                    .collect();
                let field_indices: Vec<_> = (0..fields_unnamed.unnamed.len())
                    .map(syn::Index::from)
                    .collect();
                struct_defs.push(quote! {
                    #vis struct #struct_name(#(#field_defs),*);
                });
                from_impls.push(quote! {
                    impl ::core::convert::From<#struct_name> for #enum_name {
                        fn from(v: #struct_name) -> Self {
                            #enum_name::#variant_name(#(v.#field_indices),*)
                        }
                    }
                });
            }
        }

        event_impls.push(quote! {
            impl ::egui_event::Event for #struct_name {}
        });
    }

    // The enum itself also implements Event.
    let enum_event_impl = quote! {
        impl ::egui_event::Event for #enum_name {}
    };

    let expanded = quote! {
        #(#struct_defs)*
        #(#event_impls)*
        #enum_event_impl
        #(#from_impls)*
    };

    expanded.into()
}
