// This file is adapted from MIRAI (https://github.com/facebookexperimental/MIRAI)
// Original author: Herman Venter <hermanv@fb.com>
// Original copyright header:

// Copyright (c) Facebook, Inc. and its affiliates.
//
// This source code is licensed under the MIT license found in the
// LICENSE file in the root directory of this source tree.

use crate::analysis::memory::expression::ExpressionType;
use crate::analysis::memory::path::{Path, PathEnum, PathSelector};
use rustc_hir::def_id::DefId;
use rustc_middle::mir;
use rustc_middle::ty::ty_kind::TyKind;
use rustc_middle::ty::{
    Binder, ExistentialPredicate, ExistentialProjection, ExistentialTraitRef, FnSig, GenericArg,
    GenericArgKind, GenericArgs, GenericArgsRef, ParamTy, Ty, TyCtxt,
};
use rustc_target::abi::FieldIdx;
use std::collections::HashMap;
use std::fmt::{Debug, Formatter, Result};
use std::rc::Rc;

pub struct TypeVisitor<'tcx> {
    pub actual_argument_types: Vec<Ty<'tcx>>,
    pub def_id: DefId,
    pub generic_argument_map: Option<HashMap<rustc_span::Symbol, Ty<'tcx>>>,
    pub generic_arguments: Option<GenericArgsRef<'tcx>>,
    pub mir: mir::Body<'tcx>,
    pub path_ty_cache: HashMap<Rc<Path>, Ty<'tcx>>,
    tcx: TyCtxt<'tcx>,
}

impl<'tcx> Debug for TypeVisitor<'tcx> {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        "TypeVisitor".fmt(f)
    }
}

impl<'compilation, 'tcx> TypeVisitor<'tcx> {
    pub fn new(def_id: DefId, mir: mir::Body<'tcx>, tcx: TyCtxt<'tcx>) -> TypeVisitor<'tcx> {
        TypeVisitor {
            actual_argument_types: Vec::new(),
            def_id,
            generic_argument_map: None,
            generic_arguments: None,
            mir,
            path_ty_cache: HashMap::new(),
            tcx,
        }
    }

    // TODO: this is only used in `copy_or_move_subslice`, remove this if not necessary
    /// Returns the size in bytes (including padding) or an element of the given collection type.
    /// If the type is not a collection, it returns one.
    pub fn get_elem_type_size(&self, ty: Ty<'tcx>) -> u64 {
        match ty.kind() {
            TyKind::Array(ty, _) | TyKind::Slice(ty) => self.get_type_size(*ty),
            TyKind::RawPtr(t, _) => self.get_type_size(*t),
            _ => 1,
        }
    }

    /// Returns a parameter environment for the current function.
    pub fn get_param_env(&self) -> rustc_middle::ty::ParamEnv<'tcx> {
        self.tcx.param_env(self.def_id)
    }

    /// This is a hacky and brittle way to navigate the Rust compiler's type system.
    /// Eventually it should be replaced with a comprehensive and principled mapping.
    pub fn get_path_rustc_type(
        &mut self,
        path: &Rc<Path>,
        current_span: rustc_span::Span,
    ) -> Ty<'tcx> {
        if let Some(ty) = self.path_ty_cache.get(path) {
            return *ty;
        }
        match &path.value {
            PathEnum::LocalVariable { ordinal } => {
                if *ordinal > 0 && *ordinal < self.mir.local_decls.len() {
                    self.mir.local_decls[mir::Local::from(*ordinal)].ty
                } else {
                    info!("path.value is {:?}", path.value);
                    self.tcx.types.unit
                }
            }
            PathEnum::Parameter { ordinal } => {
                if self.actual_argument_types.len() >= *ordinal {
                    self.actual_argument_types[*ordinal - 1]
                } else if *ordinal > 0 && *ordinal < self.mir.local_decls.len() {
                    self.mir.local_decls[mir::Local::from(*ordinal)].ty
                } else {
                    info!("path.value is {:?}", path.value);
                    self.tcx.types.unit
                }
            }
            PathEnum::Result => {
                if self.mir.local_decls.is_empty() {
                    info!("result type wanted from function without result local");
                    self.tcx.types.unit
                } else {
                    self.mir.local_decls[mir::Local::from(0usize)].ty
                }
            }
            PathEnum::QualifiedPath {
                qualifier,
                selector,
                ..
            } => {
                let t = self.get_path_rustc_type(qualifier, current_span);
                match &**selector {
                    PathSelector::Slice(_) => {
                        return t;
                    }
                    PathSelector::Field(ordinal) => {
                        let bt = Self::get_dereferenced_type(t);
                        match &bt.kind() {
                            TyKind::Adt(adt_def, substs) => {
                                let variants = adt_def.variants();
                                if !is_union(bt) {
                                    if let Some(variant_index) = variants.last_index() {
                                        let variant = &variants[variant_index];
                                        if *ordinal < variant.fields.len() {
                                            let field =
                                                &variant.fields[FieldIdx::from_usize(*ordinal)];
                                            return field.ty(self.tcx, substs);
                                        }
                                    }
                                }
                            }
                            TyKind::Closure(.., subs) => {
                                if *ordinal + 4 < subs.len() {
                                    return subs.as_ref()[*ordinal + 4].expect_ty();
                                }
                            }
                            TyKind::Tuple(types) => {
                                if let Some(gen_arg) = types.get(*ordinal as usize) {
                                    return *gen_arg;
                                }
                            }
                            _ => (),
                        }
                    }
                    PathSelector::Deref => {
                        return Self::get_dereferenced_type(t);
                    }
                    PathSelector::Discriminant => {
                        return self.tcx.types.i32;
                    }
                    // PathSelector::Downcast(_, ordinal) => {
                    //     let t = type_visitor::get_target_type(t);
                    //     if let TyKind::Adt(def, substs) = t.kind() {
                    //         use rustc_index::vec::Idx;
                    //         if *ordinal >= def.variants.len() {
                    //             debug!(
                    //                 "illegally down casting to index {} of {:?} at {:?}",
                    //                 *ordinal, t, current_span
                    //             );
                    //             return self.tcx.types.never;
                    //         }
                    //         let variant = &def.variants[VariantIdx::new(*ordinal)];
                    //         let field_tys = variant.fields.iter().map(|fd| fd.ty(self.tcx, substs));
                    //         return self.tcx.mk_tup(field_tys);
                    //     }
                    //     return self.tcx.types.never;
                    //     // if let TyKind::Adt(def, substs) = &t.kind() {
                    //     //     use rustc_index::vec::Idx;
                    //     //     let variant = &def.variants[VariantIdx::new(*ordinal)];
                    //     //     let field_tys = variant.fields.iter().map(|fd| fd.ty(self.tcx, substs));
                    //     //     return self.tcx.mk_tup(field_tys);
                    //     // }
                    // }
                    PathSelector::Index(_) => match &t.kind() {
                        TyKind::Array(elem_ty, _) | TyKind::Slice(elem_ty) => {
                            return *elem_ty;
                        }
                        _ => (),
                    },
                    _ => {}
                }
                info!("current span is {:?}", current_span);
                info!("t is {:?}", t);
                info!("qualifier is {:?}", qualifier);
                info!("selector is {:?}", selector);
                self.tcx.types.unit
            }
            PathEnum::StaticVariable { def_id, .. } => {
                if let Some(def_id) = def_id {
                    return self.tcx.type_of(*def_id).instantiate_identity();
                }
                info!("path.value is {:?}", path.value);
                self.tcx.types.unit
            }
            _ => {
                info!("path.value is {:?}", path.value);
                self.tcx.types.unit
            }
        }
    }

    /// Returns the target type of a reference type.
    fn get_dereferenced_type(ty: Ty<'tcx>) -> Ty<'tcx> {
        match &ty.kind() {
            TyKind::Ref(_, t, _) => *t,
            _ => ty,
        }
    }

    /// If Operand corresponds to a compile time constant function, return
    /// the generic parameter substitutions (type arguments) that are used by
    /// the call instruction whose operand this is.
    pub fn get_generic_arguments_map(
        &self,
        def_id: DefId,
        generic_args: GenericArgsRef<'tcx>,
        actual_argument_types: &[Ty<'tcx>],
    ) -> Option<HashMap<rustc_span::Symbol, Ty<'tcx>>> {
        let mut substitution_map = self.generic_argument_map.clone();
        let mut map: HashMap<rustc_span::Symbol, Ty<'tcx>> = HashMap::new();

        // This iterates over the callee's generic parameter definitions.
        // If the parent of the callee is generic, those definitions are iterated
        // as well. This applies recursively. Note that a child cannot mask the
        // generic parameters of its parent with one of its own, so each parameter
        // definition in this iteration will have a unique name.
        GenericArgs::for_item(self.tcx, def_id, |param_def, _| {
            if let Some(gen_arg) = generic_args.get(param_def.index as usize) {
                if let GenericArgKind::Type(ty) = gen_arg.unpack() {
                    let specialized_gen_arg_ty =
                        self.specialize_generic_argument_type(ty, &substitution_map);
                    if let Some(substitution_map) = &mut substitution_map {
                        substitution_map.insert(param_def.name, specialized_gen_arg_ty);
                    }
                    map.insert(param_def.name, specialized_gen_arg_ty);
                }
            } else {
                debug!("unmapped generic param def");
            }
            self.tcx.mk_param_from_def(param_def) // not used
        });
        // Add "Self" -> actual_argument_types[0]
        if let Some(self_ty) = actual_argument_types.get(0) {
            let self_ty = if let TyKind::Ref(_, ty, _) = self_ty.kind() {
                ty
            } else {
                self_ty
            };
            let self_sym = rustc_span::Symbol::intern("Self");
            map.entry(self_sym).or_insert(*self_ty);
        }
        if map.is_empty() {
            None
        } else {
            Some(map)
        }
    }

    /// Returns an ExpressionType value corresponding to the Rustc type of the place.
    pub fn get_place_type(
        &mut self,
        place: &mir::Place<'tcx>,
        current_span: rustc_span::Span,
    ) -> ExpressionType {
        (self.get_rustc_place_type(place, current_span).kind()).into()
    }

    /// Returns the rustc Ty of the given place in memory.
    pub fn get_rustc_place_type(
        &self,
        place: &mir::Place<'tcx>,
        current_span: rustc_span::Span,
    ) -> Ty<'tcx> {
        let result = {
            let base_type = self.mir.local_decls[place.local].ty;
            self.get_type_for_projection_element(current_span, base_type, &place.projection)
        };
        match result.kind() {
            // Type parameter, e.g., `T` in `fn f<T>(x: T) {}`
            TyKind::Param(t_par) => {
                if let Some(generic_args) = self.generic_arguments {
                    if let Some(gen_arg) = generic_args.as_ref().get(t_par.index as usize) {
                        return gen_arg.expect_ty();
                    }
                    if t_par.name.as_str() == "Self" && !self.actual_argument_types.is_empty() {
                        return self.actual_argument_types[0];
                    }
                }
            }
            TyKind::Ref(region, ty, mutbl) => {
                if let TyKind::Param(t_par) = ty.kind() {
                    if t_par.name.as_str() == "Self" && !self.actual_argument_types.is_empty() {
                        return self.tcx.mk_ty_from_kind(TyKind::Ref(
                            *region,
                            self.actual_argument_types[0],
                            *mutbl,
                        ));
                    }
                }
            }
            _ => {}
        }
        result
    }

    /// Returns the rustc TyKind of the element selected by projection_elem.
    pub fn get_type_for_projection_element(
        &self,
        current_span: rustc_span::Span,
        base_ty: Ty<'tcx>,
        place_projection: &[rustc_middle::mir::PlaceElem<'tcx>],
    ) -> Ty<'tcx> {
        place_projection
            .iter()
            .fold(base_ty, |base_ty, projection_elem| match projection_elem {
                mir::ProjectionElem::Deref => match &base_ty.kind() {
                    TyKind::Adt(..) => base_ty,
                    TyKind::RawPtr(ty, _) => *ty,
                    TyKind::Ref(_, ty, _) => *ty,
                    _ => {
                        debug!(
                            "span: {:?}\nelem: {:?} type: {:?}",
                            current_span, projection_elem, base_ty
                        );
                        unreachable!();
                    }
                },
                mir::ProjectionElem::Field(_, ty) => *ty,
                mir::ProjectionElem::Index(_)
                | mir::ProjectionElem::ConstantIndex { .. }
                | mir::ProjectionElem::Subslice { .. } => match &base_ty.kind() {
                    TyKind::Adt(..) => base_ty,
                    TyKind::Array(ty, _) => *ty,
                    TyKind::Ref(_, ty, _) => get_element_type(*ty),
                    TyKind::Slice(ty) => *ty,
                    _ => {
                        debug!(
                            "span: {:?}\nelem: {:?} type: {:?}",
                            current_span, projection_elem, base_ty
                        );
                        unreachable!();
                    }
                },
                mir::ProjectionElem::Downcast(..) => base_ty,
                mir::ProjectionElem::OpaqueCast(_) => todo!(),
                mir::ProjectionElem::Subtype(_) => todo!(),
            })
    }

    /// Returns the size in bytes (including padding) of an instance of the given type.
    pub fn get_type_size(&self, ty: Ty<'tcx>) -> u64 {
        let param_env = self.get_param_env();
        if let Ok(ty_and_layout) = self.tcx.layout_of(param_env.and(ty)) {
            ty_and_layout.layout.size.bytes()
        } else {
            0
        }
    }

    fn specialize_generic_argument(
        &self,
        gen_arg: GenericArg<'tcx>,
        map: &Option<HashMap<rustc_span::Symbol, Ty<'tcx>>>,
    ) -> GenericArg<'tcx> {
        match gen_arg.unpack() {
            GenericArgKind::Type(ty) => self.specialize_generic_argument_type(ty, map).into(),
            _ => gen_arg,
        }
    }

    pub fn specialize_generic_argument_type(
        &self,
        gen_arg_type: Ty<'tcx>,
        map: &Option<HashMap<rustc_span::Symbol, Ty<'tcx>>>,
    ) -> Ty<'tcx> {
        if map.is_none() {
            return gen_arg_type;
        }
        match gen_arg_type.kind() {
            TyKind::Adt(..) => gen_arg_type,
            TyKind::Array(elem_ty, len) => {
                let specialized_elem_ty = self.specialize_generic_argument_type(*elem_ty, map);
                self.tcx
                    .mk_ty_from_kind(TyKind::Array(specialized_elem_ty, *len))
            }
            TyKind::Slice(elem_ty) => {
                let specialized_elem_ty = self.specialize_generic_argument_type(*elem_ty, map);
                self.tcx.mk_ty_from_kind(TyKind::Slice(specialized_elem_ty))
            }
            TyKind::RawPtr(ty, mutbl) => {
                let specialized_ty = self.specialize_generic_argument_type(*ty, map);
                self.tcx
                    .mk_ty_from_kind(TyKind::RawPtr(specialized_ty, *mutbl))
            }
            TyKind::Ref(region, ty, mutbl) => {
                let specialized_ty = self.specialize_generic_argument_type(*ty, map);
                self.tcx
                    .mk_ty_from_kind(TyKind::Ref(*region, specialized_ty, *mutbl))
            }
            TyKind::FnDef(def_id, substs) => self
                .tcx
                .mk_ty_from_kind(TyKind::FnDef(*def_id, self.specialize_substs(substs, map))),
            TyKind::FnPtr(fn_sig) => {
                let map_fn_sig = |fn_sig: FnSig<'tcx>| {
                    let specialized_inputs_and_output: Vec<_> = fn_sig
                        .inputs_and_output
                        .iter()
                        .map(|ty| self.specialize_generic_argument_type(ty, map))
                        .collect();
                    let specialized_inputs_and_output = self
                        .tcx
                        .mk_type_list(specialized_inputs_and_output.as_ref());
                    FnSig {
                        inputs_and_output: specialized_inputs_and_output,
                        c_variadic: fn_sig.c_variadic,
                        unsafety: fn_sig.unsafety,
                        abi: fn_sig.abi,
                    }
                };
                let specialized_fn_sig = fn_sig.map_bound(map_fn_sig);
                self.tcx.mk_ty_from_kind(TyKind::FnPtr(specialized_fn_sig))
            }
            TyKind::Dynamic(predicates, region, dynkind) => {
                let map_predicates = |predicates: &'tcx rustc_middle::ty::List<
                    Binder<ExistentialPredicate<'tcx>>,
                >| {
                    let eps: Vec<_> = predicates
                        .iter()
                        .map(
                            |pred: Binder<ExistentialPredicate<'tcx>>| match pred.skip_binder() {
                                ExistentialPredicate::Trait(ExistentialTraitRef {
                                    def_id,
                                    args,
                                }) => {
                                    pred.rebind(ExistentialPredicate::Trait(ExistentialTraitRef {
                                        def_id,
                                        args: self.specialize_substs(args, map),
                                    }))
                                }
                                ExistentialPredicate::Projection(ExistentialProjection {
                                    def_id,
                                    args,
                                    term,
                                }) => pred.rebind(ExistentialPredicate::Projection(
                                    ExistentialProjection {
                                        def_id,
                                        args: self.specialize_substs(args, map),
                                        term: term,
                                    },
                                )),
                                ExistentialPredicate::AutoTrait(_) => pred,
                            },
                        )
                        .collect();
                    self.tcx.mk_poly_existential_predicates(eps.as_slice())
                };
                let specialized_predicates = map_predicates(predicates);
                // let specialized_predicates = predicates.map_bound(map_predicates);
                self.tcx
                    .mk_ty_from_kind(TyKind::Dynamic(specialized_predicates, *region, *dynkind))
            }
            TyKind::Tuple(substs) => {
                let specialized_substs = substs
                    .iter()
                    .map(|sub_type| self.specialize_generic_argument_type(sub_type, map));
                self.tcx.mk_ty_from_kind(TyKind::Tuple(
                    self.tcx.mk_type_list_from_iter(specialized_substs),
                ))
            }
            TyKind::Param(ParamTy { name, .. }) => {
                if let Some(ty) = map.as_ref().unwrap().get(&name) {
                    return *ty;
                }
                gen_arg_type
            }
            _ => gen_arg_type,
        }
    }

    pub fn specialize_substs(
        &self,
        substs: GenericArgsRef<'tcx>,
        map: &Option<HashMap<rustc_span::Symbol, Ty<'tcx>>>,
    ) -> GenericArgsRef<'tcx> {
        let specialized_generic_args = substs
            .iter()
            .map(|gen_arg| self.specialize_generic_argument(gen_arg, &map));
        self.tcx.mk_args_from_iter(specialized_generic_args)
    }

    // TODO: this is only used in promote constant, remove it if not necessary
    pub fn starts_with_slice_pointer(&self, ty_kind: &TyKind<TyCtxt<'tcx>>) -> bool {
        match ty_kind {
            TyKind::RawPtr(ty, _) => {
                // Pointers to sized arrays are thin pointers.
                matches!(ty.kind(), TyKind::Slice(..))
            }
            TyKind::Ref(_, target, _) => {
                // Pointers to sized arrays are thin pointers.
                matches!(target.kind(), TyKind::Slice(..))
            }
            TyKind::Adt(def, substs) => {
                for v in def.variants().iter() {
                    if let Some(field0) = v.fields.get(FieldIdx::from_usize(0)) {
                        let field0_ty = field0.ty(self.tcx, substs);
                        if self.starts_with_slice_pointer(&field0_ty.kind()) {
                            return true;
                        }
                    }
                }
                false
            }
            TyKind::Tuple(substs) => {
                if let Some(field0_ty) = substs.iter().map(|s| s).next() {
                    self.starts_with_slice_pointer(field0_ty.kind())
                } else {
                    false
                }
            }
            _ => false,
        }
    }
}

/// Returns the element type of an array or slice type.
pub fn get_element_type(ty: Ty<'_>) -> Ty<'_> {
    match &ty.kind() {
        TyKind::Array(t, _) => *t,
        TyKind::Ref(_, t, _) => match &t.kind() {
            TyKind::Array(t, _) => *t,
            TyKind::Slice(t) => *t,
            _ => *t,
        },
        TyKind::Slice(t) => *t,
        _ => ty,
    }
}

/// Returns true if the ty is a union.
pub fn is_union(ty: Ty<'_>) -> bool {
    if let TyKind::Adt(def, ..) = ty.kind() {
        def.is_union()
    } else {
        false
    }
}

pub fn get_target_type(ty: Ty<'_>) -> Ty<'_> {
    match ty.kind() {
        TyKind::RawPtr(t, _) | TyKind::Ref(_, t, _) => *t,
        _ => ty,
    }
}

pub fn is_slice_pointer<'tcx>(ty_kind: &TyKind<TyCtxt<'tcx>>) -> bool {
    if let TyKind::RawPtr(target, _) | TyKind::Ref(_, target, _) = ty_kind {
        // Pointers to sized arrays and slice pointers are thin pointers.
        matches!(target.kind(), TyKind::Slice(..) | TyKind::Str)
    } else {
        false
    }
}
