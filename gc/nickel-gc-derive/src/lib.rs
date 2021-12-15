/// This was a copy from sundial-gc-derive.
/// It still needs to be customised
///
use proc_macro2::{Span, TokenStream};
use quote::quote;
use syn::{
    parse_macro_input, parse_quote, punctuated::Punctuated, token::Comma, DataEnum, DataStruct,
    DeriveInput, Field, Fields, FieldsNamed, FieldsUnnamed, Ident, Type, Variant,
};

#[proc_macro_derive(GC)]
pub fn derive_trace_impl(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    proc_macro::TokenStream::from(trace_impl(input))
}

// TODO handle associated type constraints of the form List<'r, T: 'r>.
// The work around is to use a where clause where T: 'r
fn trace_impl(input: DeriveInput) -> TokenStream {
    let DeriveInput {
        ident: top_name,
        mut generics,
        data,
        ..
    } = input;

    generics.make_where_clause();
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    let (impl_generics_l, _, _) = generics.split_for_impl();

    let mut where_clause = where_clause.unwrap().clone();
    let mut where_clause_l = where_clause.clone();

    generics.type_params().for_each(|t| {
        where_clause
            .predicates
            .push(parse_quote! { #t: nickel_gc::AsStatic });

        where_clause_l
            .predicates
            .push(parse_quote! { #t: nickel_gc::AsStatic });
    });

    let tuple = |unnamed: Punctuated<Field, Comma>, types: &mut Vec<Type>| {
        let args: Vec<_> = unnamed
            .iter()
            .enumerate()
            .map(|(i, Field { ty, .. })| {
                let i = Ident::new(&format!("f{}", i), Span::call_site());
                let arg = quote! {#i};
                types.push(ty.clone());
                arg
            })
            .collect();

        let e = quote! {
            #(GC::evacuate(#args, direct_gc_ptrs); )*
        };

        e
    };

    let struc = |named: Punctuated<Field, Comma>, types: &mut Vec<Type>| {
        let args: Vec<_> = named
            .iter()
            .map(|Field { ty, ident, .. }| {
                let ident = ident.as_ref().unwrap();
                let arg = quote! {#ident};
                types.push(ty.clone());
                arg
            })
            .collect();

        let e = quote! {
            #(GC::evacuate(#args, direct_gc_ptrs); )*
        };

        e
    };

    let tuple_names = |unnamed: &Punctuated<Field, Comma>| -> Vec<_> {
        unnamed
            .iter()
            .enumerate()
            .map(|(i, _)| Ident::new(&format!("f{}", i), Span::call_site()))
            .map(|i| quote! {#i})
            .collect()
    };

    let struc_names = |named: &Punctuated<Field, Comma>| -> Vec<_> {
        named
            .iter()
            .map(|Field { ident, .. }| ident.clone().unwrap())
            .collect()
    };

    let mut types: Vec<Type> = vec![];

    let evacuate = match data {
        syn::Data::Struct(DataStruct { fields, .. }) => match fields {
            syn::Fields::Named(FieldsNamed { named, .. }) => {
                let names = struc_names(&named);
                let e = struc(named, &mut types);

                let e = quote! {
                    let Self {#(#names, )*} = s;
                    #e
                };
                e
            }
            syn::Fields::Unnamed(FieldsUnnamed { unnamed, .. }) => {
                let _names = tuple_names(&unnamed);
                let e = tuple(unnamed, &mut types);
                let e = quote! {
                    #e
                };
                e
            }
            syn::Fields::Unit => quote! {},
        },

        syn::Data::Enum(DataEnum { variants, .. }) => {
            let e_arms: Vec<_> = variants
                .into_iter()
                .filter_map(|Variant { ident, fields, .. }| match fields {
                    Fields::Named(FieldsNamed { named, .. }) => {
                        let names = struc_names(&named);
                        let e = struc(named, &mut types);

                        let e = quote! {
                            #top_name::#ident{#(#names, )*} => {
                                #e
                            },
                        };

                        Some(e)
                    }
                    Fields::Unnamed(FieldsUnnamed { unnamed, .. }) => {
                        let names = tuple_names(&unnamed);
                        let e = tuple(unnamed, &mut types);

                        let e = quote! {
                            #top_name::#ident(#(#names, )*) => {
                                #e
                            },
                        };

                        Some(e)
                    }
                    Fields::Unit => None,
                })
                .collect();

            let e = quote! {
                match s {
                    #(#e_arms)*
                    _ => (),
                }
            };

            e
        }
        syn::Data::Union(_) => panic!("Cannot derive GC for a Union"),
    };

    let type_params: Vec<_> = generics
        .type_params()
        .map(|t| t.ident.clone())
        .map(|t| quote! { #t::Static })
        .collect();

    quote! {
        unsafe impl #impl_generics nickel_gc::GC for #top_name #ty_generics #where_clause {
            fn trace(s: &self, direct_gc_ptrs: &mut Vec<nickel_gc::internals::TraceAt>) {
                #evacuate
            }
        }

       unsafe impl #impl_generics_l nickel_gc::AsStatic for #top_name #ty_generics #where_clause_l {
            type Static = #top_name<'static, #(#type_params,)*>;
       }
    }
}

#[test]
fn binary_tree_derive_test() {
    let input: DeriveInput = parse_quote! {
         pub enum BinaryTree<'r, K, V> {
             Empty,
             Branch(Gc<'r, (K, Self, Self, V)>),
         }
    };

    let ts = trace_impl(input);
    eprintln!("{}", ts);
}

#[test]
fn add_to_where() {
    let mut w: syn::WhereClause = parse_quote! { where };
    let t: Ident = parse_quote! { T };
    w.predicates.push(parse_quote! { #t: nickel_gc::GC });
    w.predicates.push(parse_quote! { #t: nickel_gc::GC });
}
