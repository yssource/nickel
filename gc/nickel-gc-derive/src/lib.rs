/// This was a copy from sundial-gc-derive.
/// It still needs to be customised
///
use proc_macro2::{Ident, Span, TokenStream};
use quote::{quote, ToTokens};
use syn::{
    parse_macro_input, parse_quote, punctuated::Punctuated, token::Comma, DataEnum, DataStruct,
    DeriveInput, Field, Fields, FieldsNamed, FieldsUnnamed, Type, Variant,
};

#[proc_macro_derive(GC, attributes(unsafe_impl_gc_static))]
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
        let b = &t.bounds;
        let t = &t.ident;

        where_clause.predicates.push(parse_quote! { #t: #b });
        where_clause
            .predicates
            .push(parse_quote! { #t: nickel_gc::GC });

        where_clause_l.predicates.push(parse_quote! { #t: #b });
        where_clause_l
            .predicates
            .push(parse_quote! { #t::Static: #b });
        where_clause_l
            .predicates
            .push(parse_quote! { #t: nickel_gc::AsStatic });
    });

    let tuple = |unnamed: Punctuated<Field, Comma>, types: &mut Vec<Type>| {
        let args: Vec<_> = unnamed
            .iter()
            .filter(|f| {
                // This is hacky
                !f.attrs.iter().any(|attr| {
                    attr.to_token_stream()
                        .to_string()
                        .contains("unsafe_impl_gc_static")
                })
            })
            .enumerate()
            .map(|(i, Field { ty, .. })| {
                let i = Ident::new(&format!("f{}", i), Span::call_site());
                let arg = quote! {#i};
                types.push(ty.clone());
                arg
            })
            .collect();

        let e = quote! {
            #(nickel_gc::GC::track(#args, |#args| nickel_gc::GC::trace(#args, direct_gc_ptrs)); )*
        };

        e
    };

    let struc = |named: Punctuated<Field, Comma>, types: &mut Vec<Type>| {
        let args: Vec<_> = named
            .iter()
            .filter(|f| {
                // This is hacky
                !f.attrs.iter().any(|attr| {
                    attr.to_token_stream()
                        .to_string()
                        .contains("unsafe_impl_gc_static")
                })
            })
            .map(|Field { ty, ident, .. }| {
                let ident = ident.as_ref().unwrap();
                let arg = quote! {#ident};
                types.push(ty.clone());
                arg
            })
            .collect();

        let e = quote! {
            #(nickel_gc::GC::trace(#args, direct_gc_ptrs); )*
        };

        e
    };

    let tuple_names = |unnamed: &Punctuated<Field, Comma>| -> Vec<_> {
        unnamed
            .iter()
            .filter(|f| {
                // This is hacky
                !f.attrs.iter().any(|attr| {
                    attr.to_token_stream()
                        .to_string()
                        .contains("unsafe_impl_gc_static")
                })
            })
            .enumerate()
            .map(|(i, _)| Ident::new(&format!("f{}", i), Span::call_site()))
            .map(|i| quote! {#i})
            .collect()
    };

    let struc_names = |named: &Punctuated<Field, Comma>| -> Vec<_> {
        named
            .iter()
            .filter(|f| {
                // This is hacky
                !f.attrs.iter().any(|attr| {
                    attr.to_token_stream()
                        .to_string()
                        .contains("unsafe_impl_gc_static")
                })
            })
            .map(|Field { ident, .. }| ident.clone().unwrap())
            .collect()
    };

    let mut types: Vec<Type> = vec![];

    let trace = match data {
        syn::Data::Struct(DataStruct { fields, .. }) => match fields {
            syn::Fields::Named(FieldsNamed { named, .. }) => {
                let names = struc_names(&named);
                let e = struc(named, &mut types);

                let e = quote! {
                    let Self {#(#names, )* ..} = s;
                    #e
                };
                e
            }
            syn::Fields::Unnamed(FieldsUnnamed { unnamed, .. }) => {
                let names = tuple_names(&unnamed);
                let e = tuple(unnamed, &mut types);

                let e = quote! {
                    let Self (#(#names, )* ..) = s;
                    #e
                };
                e
            }
            syn::Fields::Unit => quote! {},
        },

        syn::Data::Enum(DataEnum { variants, .. }) => {
            let e_arms: Vec<_> = variants
                .into_iter()
                .filter(|f| {
                    // This is hacky
                    !f.attrs.iter().any(|attr| {
                        attr.to_token_stream()
                            .to_string()
                            .contains("unsafe_impl_gc_static")
                    })
                })
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

    let stat = if generics.lifetimes().count() == 0 {
        quote! {#top_name<#(#type_params,)*>}
    } else {
        quote! { #top_name<'static, #(#type_params,)*> }
    };

    quote! {
        unsafe impl #impl_generics nickel_gc::GC for #top_name #ty_generics #where_clause {
            unsafe fn trace(s: &Self, direct_gc_ptrs: *mut Vec<()>) {
                #trace
            }
        }

       impl #impl_generics_l nickel_gc::AsStatic for #top_name #ty_generics #where_clause_l {
            type Static = #stat;
       }
    }
}

#[test]
fn binary_tree_derive_test() {
    let input: DeriveInput = parse_quote! {
         pub enum BinaryTree<'r, K, V> {
            #[unsafe_impl_gc_static]
             Empty,
             Branch(Gc<'r, (K, Self, Self, V)>),
         }
    };

    let ts = trace_impl(input);
    eprintln!("{}", ts);
}

#[test]
fn list_derive_test() {
    let input: DeriveInput = parse_quote! {
        struct List<'g, T> {
            #[unsafe_impl_gc_static]
            elm: T,
            next: Option<Gc<'g, List<'g, T>>>,
        }
    };

    let ts = trace_impl(input);
    eprintln!("{}", ts);
}

#[test]
fn counted_derive_test() {
    let input: DeriveInput = parse_quote! {
        struct Counted(&'static AtomicIsize);
    };

    let ts = trace_impl(input);
    eprintln!("{}", ts);
}

#[test]
fn term_derive_test() {
    let input: DeriveInput = parse_quote! {
        enum Term {
            Null,
            Bool(bool),
            Fun(Ident, Gc<Term>),
            Struc{name: String, fields: Map<String, Term>},
            #[unsafe_impl_static]
            Foo(Foo),
        }
    };

    let ts = trace_impl(input);
    eprintln!("{}", ts);
}

#[test]
fn inner_thunk_data_derive_test() {
    let input: DeriveInput = parse_quote! {
        pub enum InnerThunkData {
            Standard(Closure),
            Reversible {
                orig: Rc<Closure>,
                cached: Rc<Closure>,
            },
        }
    };

    let ts = trace_impl(input);
    eprintln!("{}", ts);
}

#[test]
fn environment_derive_test() {
    let input: DeriveInput = parse_quote! {
        #[derive(Debug, PartialEq, Default, GC)]
        pub struct Environment<K: Hash + Eq, V: PartialEq> {
            #[unsafe_impl_gc_static]
            current: Rc<HashMap<K, V>>,
            #[unsafe_impl_gc_static]
            previous: RefCell<Option<Rc<Environment<K, V>>>>,
        }
    };

    let ts = trace_impl(input);
    eprintln!("{}", ts);
}

#[test]
fn add_to_where() {
    let mut w: syn::WhereClause = parse_quote! { where };
    let t: proc_macro2::Ident = parse_quote! { T };
    w.predicates.push(parse_quote! { #t: nickel_gc::GC });
    w.predicates.push(parse_quote! { #t: nickel_gc::GC });
}
