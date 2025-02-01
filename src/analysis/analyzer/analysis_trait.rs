use crate::analysis::abstract_domain::AbstractDomain;
use crate::analysis::analysis_result::{AnalysisInfo, Result};
use crate::analysis::global_context::GlobalContext;
use crate::analysis::numerical::apron_domain::{
    ApronAbstractDomain, ApronDomainType, GetManagerTrait,
};
use rustc_hir::def_id::DefId;

/// General trait for static analysis
/// Developers may reuse this trait to implement their own analysis
pub trait StaticAnalysis<'tcx, 'compiler> {
    fn new(context: GlobalContext<'tcx, 'compiler>) -> Self;
    fn run(self) -> Result<AnalysisInfo>;
    fn analyze_function<DomainType>(
        &mut self,
        def_id: DefId,
        abstract_domain: AbstractDomain<DomainType>,
    ) where
        DomainType: ApronDomainType,
        ApronAbstractDomain<DomainType>: GetManagerTrait;
    fn emit_diagnostics(self);
}
