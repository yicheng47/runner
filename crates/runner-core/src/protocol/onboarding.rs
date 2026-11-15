use crate::runtime::Runtime;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OnboardingStatus {
    pub completed: bool,
    pub completed_at: Option<String>,
    pub has_crews: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StarterCrewSlotProposal {
    pub handle: String,
    pub display_name: String,
    pub runtime: Runtime,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StarterCrewProposal {
    pub name: String,
    pub slots: Vec<StarterCrewSlotProposal>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateStarterCrewArgs {
    #[serde(default)]
    pub default_runtime: Option<Runtime>,
    #[serde(default)]
    pub enabled_runtimes: Option<Vec<Runtime>>,
}
