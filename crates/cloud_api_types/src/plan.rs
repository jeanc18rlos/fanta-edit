use serde::{Deserialize, Serialize};

use crate::{KnownOrUnknown, Timestamp};

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Plan {
    // The aliases accept the plan ids sent by the account API, which keeps
    // them stable for the 1.0.0 alpha clients already installed.
    #[default]
    #[serde(alias = "zed_free")]
    FantaFree,
    #[serde(alias = "zed_pro")]
    FantaPro,
    #[serde(alias = "zed_pro_trial")]
    FantaProTrial,
    #[serde(alias = "zed_business")]
    FantaBusiness,
    #[serde(alias = "zed_vip")]
    FantaVip,
    #[serde(alias = "zed_student")]
    FantaStudent,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct PlanInfo {
    /// We've named this field `plan_v3` to avoid breaking older clients when we start returning new plan variants.
    #[serde(rename = "plan_v3")]
    pub plan: KnownOrUnknown<Plan, String>,
    pub subscription_period: Option<SubscriptionPeriod>,
    pub usage: cloud_llm_client::CurrentUsage,
    pub trial_started_at: Option<Timestamp>,
    pub is_account_too_young: bool,
    pub has_overdue_invoices: bool,
}

impl PlanInfo {
    pub fn plan(&self) -> Plan {
        match &self.plan {
            KnownOrUnknown::Known(plan) => *plan,
            KnownOrUnknown::Unknown(_) => {
                // If we get a plan that we don't recognize, fall back to the Free plan.
                Plan::FantaFree
            }
        }
    }
}

#[derive(Debug, PartialEq, Clone, Copy, Serialize, Deserialize)]
pub struct SubscriptionPeriod {
    pub started_at: Timestamp,
    pub ended_at: Timestamp,
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use serde_json::json;

    use super::*;

    #[test]
    fn test_plan_deserialize_snake_case() {
        let plan = serde_json::from_value::<Plan>(json!("fanta_free")).unwrap();
        assert_eq!(plan, Plan::FantaFree);

        let plan = serde_json::from_value::<Plan>(json!("fanta_pro")).unwrap();
        assert_eq!(plan, Plan::FantaPro);

        let plan = serde_json::from_value::<Plan>(json!("fanta_pro_trial")).unwrap();
        assert_eq!(plan, Plan::FantaProTrial);

        let plan = serde_json::from_value::<Plan>(json!("fanta_student")).unwrap();
        assert_eq!(plan, Plan::FantaStudent);

        let plan = serde_json::from_value::<Plan>(json!("zed_pro")).unwrap();
        assert_eq!(plan, Plan::FantaPro);
    }
}
