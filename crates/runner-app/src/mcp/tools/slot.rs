use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, Content};
use rmcp::{tool, tool_router, ErrorData};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::mcp::server::RunnerMcpHandler;
use crate::ops::slot;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SlotListArgs {
    /// Crew ID.
    pub crew_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SlotIdArgs {
    /// Slot ID.
    pub slot_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateSlotArgs {
    /// Slot ID.
    pub slot_id: String,
    /// Fields to update. Omitted fields are preserved.
    pub input: slot::UpdateSlotInput,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReorderSlotsArgs {
    /// Crew ID.
    pub crew_id: String,
    /// Slot IDs in the desired final order. Must include every slot exactly once.
    pub ordered_slot_ids: Vec<String>,
}

#[tool_router(router = slot_router, vis = "pub(crate)")]
impl RunnerMcpHandler {
    #[tool(description = "List the slots for a crew, ordered by position.")]
    pub async fn slot_list(
        &self,
        Parameters(SlotListArgs { crew_id }): Parameters<SlotListArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let conn = self
            .state
            .db
            .get()
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        let slots = slot::list(&conn, &crew_id)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::json(&slots)?]))
    }

    #[tool(description = "Create a slot in a crew.")]
    pub async fn slot_create(
        &self,
        Parameters(input): Parameters<slot::CreateSlotInput>,
    ) -> Result<CallToolResult, ErrorData> {
        let mut conn = self
            .state
            .db
            .get()
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        let slot = slot::create(
            &mut conn,
            &input.crew_id,
            &input.runner_id,
            &input.slot_handle,
            input.runtime_override.as_deref(),
        )
        .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        self.state.events.emit("slot/changed", &());
        Ok(CallToolResult::success(vec![Content::json(&slot)?]))
    }

    #[tool(description = "Update a slot by ID. Omitted fields are preserved.")]
    pub async fn slot_update(
        &self,
        Parameters(UpdateSlotArgs { slot_id, input }): Parameters<UpdateSlotArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let mut conn = self
            .state
            .db
            .get()
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        let slot = slot::update(&mut conn, &slot_id, input)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        self.state.events.emit("slot/changed", &());
        Ok(CallToolResult::success(vec![Content::json(&slot)?]))
    }

    #[tool(description = "Delete a slot by ID.")]
    pub async fn slot_delete(
        &self,
        Parameters(SlotIdArgs { slot_id }): Parameters<SlotIdArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let mut conn = self
            .state
            .db
            .get()
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        slot::delete(&mut conn, &slot_id)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        self.state.events.emit("slot/changed", &());
        Ok(CallToolResult::success(vec![Content::json(
            serde_json::json!({ "deleted": true, "slot_id": slot_id }),
        )?]))
    }

    #[tool(description = "Make a slot the lead slot for its crew.")]
    pub async fn slot_set_lead(
        &self,
        Parameters(SlotIdArgs { slot_id }): Parameters<SlotIdArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let mut conn = self
            .state
            .db
            .get()
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        let slot = slot::set_lead(&mut conn, &slot_id)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        self.state.events.emit("slot/changed", &());
        Ok(CallToolResult::success(vec![Content::json(&slot)?]))
    }

    #[tool(description = "Reorder all slots in a crew.")]
    pub async fn slot_reorder(
        &self,
        Parameters(ReorderSlotsArgs {
            crew_id,
            ordered_slot_ids,
        }): Parameters<ReorderSlotsArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let mut conn = self
            .state
            .db
            .get()
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        let slots = slot::reorder(&mut conn, &crew_id, ordered_slot_ids)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        self.state.events.emit("slot/changed", &());
        Ok(CallToolResult::success(vec![Content::json(&slots)?]))
    }
}
