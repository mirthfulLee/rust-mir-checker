use rustc_errors::Diag as DiagnosticBuilder;
use rustc_hir::def_id::DefId;
use rustc_middle::mir;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::ops::Deref;

/// Define the cause of a diagnostic message
/// Used to provide user options to suppress some specific kinds of warnings
/// So that we can decrease the false-positive rate
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DiagnosticCause {
    Bitwise,    // Bit-wise overflow
    Arithmetic, // Arithmetic overflow
    Assembly,   // Inline assembly
    Comparison, // Comparison operations
    DivZero,    // Division by zero / remainder by zero
    Memory,     // Memory-safety issues
    Panic,      // Run into panic code
    Index,      // Out-of-bounds access
    Other,      // Other
}

/// Extract the cause of a diagnostic message from an assertion statement
impl<O> From<&mir::AssertKind<O>> for DiagnosticCause {
    fn from(assert_kind: &mir::AssertKind<O>) -> DiagnosticCause {
        use mir::BinOp::*;
        match assert_kind {
            mir::AssertKind::BoundsCheck { .. } => DiagnosticCause::Index,
            mir::AssertKind::Overflow(bin_op, ..) => match bin_op {
                Add | Sub | Mul | Div | Rem | AddUnchecked | SubUnchecked | MulUnchecked => {
                    DiagnosticCause::Arithmetic
                }
                Shr | Shl | BitXor | BitAnd | BitOr | ShlUnchecked | ShrUnchecked => {
                    DiagnosticCause::Bitwise
                }
                Eq | Lt | Le | Ne | Ge | Gt | Cmp => DiagnosticCause::Comparison,
                Offset => DiagnosticCause::Index,
            },
            mir::AssertKind::OverflowNeg(..) => DiagnosticCause::Arithmetic,
            mir::AssertKind::DivisionByZero(..) | mir::AssertKind::RemainderByZero(..) => {
                DiagnosticCause::DivZero
            }
            _ => DiagnosticCause::Other,
        }
    }
}

/// A diagnosis, which consists of the `DiagnosticBuilder` and more information about it
// #[derive(Clone)]
#[derive(Debug)]
pub struct Diagnostic<'compiler> {
    pub builder: DiagnosticBuilder<'compiler, ()>,
    pub is_memory_safety: bool,
    pub cause: DiagnosticCause,
}

impl Clone for Diagnostic<'_> {
    fn clone(&self) -> Self {
        let msg = match self.builder.deref().messages.get(0) {
            Some((msg, _)) => msg.as_str().unwrap_or_default().to_string(),
            None => String::new(),
        };
        let new_builder = DiagnosticBuilder::new(self.builder.dcx, self.builder.level(), msg);
        Self {
            builder: new_builder,
            is_memory_safety: self.is_memory_safety,
            cause: self.cause.clone(),
        }
    }
}

impl<'compiler> Diagnostic<'compiler> {
    pub fn new(
        builder: DiagnosticBuilder<'compiler, ()>,
        is_memory_safety: bool,
        cause: DiagnosticCause,
    ) -> Self {
        Self {
            builder,
            is_memory_safety,
            cause,
        }
    }

    pub fn cancel(self) {
        self.builder.cancel();
    }

    pub fn emit(self) {
        self.builder.emit();
    }

    pub fn compare(x: &Diagnostic<'compiler>, y: &Diagnostic<'compiler>) -> Ordering {
        if x.builder
            .span
            .primary_spans()
            .lt(&y.builder.span.primary_spans())
        {
            Ordering::Less
        } else if x
            .builder
            .span
            .primary_spans()
            .gt(&y.builder.span.primary_spans())
        {
            Ordering::Greater
        } else {
            Ordering::Equal
        }
    }
}

/// Store all the diagnoses generated for each `DefId`
pub struct DiagnosticsForDefId<'compiler> {
    pub map: HashMap<DefId, Vec<Diagnostic<'compiler>>>,
    marker: PhantomData<&'compiler ()>,
}

impl<'compiler> Default for DiagnosticsForDefId<'compiler> {
    fn default() -> Self {
        Self {
            map: HashMap::new(),
            marker: PhantomData,
        }
    }
}

impl<'compiler> DiagnosticsForDefId<'compiler> {
    pub fn insert(&mut self, id: DefId, diags: Vec<Diagnostic<'compiler>>) {
        self.map.insert(id, diags);
    }
}
