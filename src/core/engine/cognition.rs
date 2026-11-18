// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0

use super::*;

impl WarmCore {
    pub fn start_cognition(&self) -> Result<()> {
        if self.cognition_running() {
            return Err(anyhow::anyhow!("cognition loop is already running"));
        }
        let atom_mem = self
            .atom_memory
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("meaning memory not enabled"))?;
        let tri_store = self
            .triple_store
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("meaning memory not enabled"))?;

        let cl = CognitionLoop::start(
            Arc::clone(atom_mem),
            Arc::clone(tri_store),
            self.cognition_loop_config(),
        );

        *self.cognition_loop.lock() = Some(cl);
        Ok(())
    }

    pub fn stop_cognition(&self) {
        if let Some(ref mut cl) = *self.cognition_loop.lock() {
            cl.stop();
        }
    }

    pub fn cognition_running(&self) -> bool {
        self.cognition_loop
            .lock()
            .as_ref()
            .is_some_and(|cl| cl.state().is_running())
    }

    pub fn cognition_cycle_count(&self) -> u64 {
        self.cognition_loop
            .lock()
            .as_ref()
            .map_or(0, |cl| cl.state().cycle_count())
    }

    pub fn take_insights(&self) -> Vec<Insight> {
        self.cognition_loop
            .lock()
            .as_ref()
            .map_or_else(Vec::new, |cl| cl.state().take_insights())
    }

    pub fn cognition_insight_count(&self) -> usize {
        self.cognition_loop
            .lock()
            .as_ref()
            .map_or(0, |cl| cl.state().insight_count())
    }

    pub(super) fn cognition_loop_config(&self) -> CognitionLoopConfig {
        let cc = &self.config.cognition;
        CognitionLoopConfig {
            interval: std::time::Duration::from_secs(cc.interval_secs),
            min_pattern_freq: cc.min_pattern_freq,
            min_abstraction_members: cc.min_abstraction_members,
            min_shared_relations: cc.min_shared_relations,
            min_peer_coverage: cc.min_peer_coverage,
            hypothesis_beta: cc.hypothesis_beta,
            min_hypothesis_confidence: cc.min_hypothesis_confidence,
            min_analogy_relations: cc.min_analogy_relations,
        }
    }

    pub fn run_cognition_once(&self) -> Vec<Insight> {
        let (atom_mem, tri_store) = match (&self.atom_memory, &self.triple_store) {
            (Some(a), Some(t)) => (a, t),
            _ => return Vec::new(),
        };
        let cfg = self.cognition_loop_config();
        CognitionLoop::run_once(atom_mem, tri_store, &cfg)
    }

    pub fn govern_memory(&self) -> Result<GovernanceReport> {
        self.govern_memory_with_limits(super::super::query_limits::QueryLimits::default())
    }

    pub fn cognition_enabled(&self) -> bool {
        self.config.cognition.enabled
    }

    pub fn add_goal(
        &self,
        name: &str,
        description: &str,
        relevance: f64,
        urgency: f64,
        cost: f64,
    ) -> Option<usize> {
        let goal_store = self.goal_store.as_ref()?;
        let atom_mem = self.atom_memory.as_ref()?;
        let (_, vec) = atom_mem.get_or_insert(name);
        Some(goal_store.add(super::super::agency::goals::Goal {
            name: name.to_string(),
            description: description.to_string(),
            vector: vec,
            relevance,
            urgency,
            cost,
            active: true,
        }))
    }

    pub fn deactivate_goal(&self, name: &str) -> bool {
        self.goal_store
            .as_ref()
            .is_some_and(|gs| gs.deactivate(name))
    }

    pub fn active_goals(&self) -> Vec<(String, f64)> {
        self.goal_store.as_ref().map_or_else(Vec::new, |gs| {
            gs.prioritized()
                .iter()
                .map(|g| (g.name.clone(), g.utility()))
                .collect()
        })
    }

    pub fn plan_goal(&self, goal: &str, causal_relations: &[&str], max_depth: usize) -> Plan {
        let tri = match &self.triple_store {
            Some(t) => t,
            None => {
                return Plan {
                    goal: goal.to_string(),
                    actions: Vec::new(),
                    complete: false,
                    total_cost: 0.0,
                }
            }
        };
        Planner::backward_chain(tri, goal, causal_relations, max_depth)
    }

    pub fn generate_questions(&self) -> Vec<Question> {
        let (atom_mem, tri_store, goal_store) =
            match (&self.atom_memory, &self.triple_store, &self.goal_store) {
                (Some(a), Some(t), Some(g)) => (a, t, g),
                _ => return Vec::new(),
            };

        let cc = &self.config.cognition;
        let gaps = super::super::cognition::gaps::GapDetector::detect(
            tri_store,
            cc.min_shared_relations,
            cc.min_peer_coverage,
        );
        let hypotheses = super::super::cognition::hypothesis::HypothesisEngine::propose(
            &gaps,
            tri_store,
            atom_mem,
            cc.hypothesis_beta,
            cc.min_hypothesis_confidence,
        );

        let mut questions = Vec::new();
        questions.extend(QuestionGenerator::from_gaps(&gaps, atom_mem, goal_store));
        questions.extend(QuestionGenerator::from_hypotheses(
            &hypotheses,
            atom_mem,
            goal_store,
        ));
        QuestionGenerator::prioritize(questions)
    }

    pub fn propose_rule(
        &self,
        name: &str,
        input_relations: Vec<String>,
        output_relation: &str,
        reason: &str,
    ) -> Option<usize> {
        let sm = self.self_modifier.as_ref()?;
        Some(sm.propose(
            ProposalKind::AddRule {
                name: name.to_string(),
                input_relations,
                output_relation: output_relation.to_string(),
            },
            reason.to_string(),
        ))
    }

    pub fn approve_proposal(&self, id: usize) -> bool {
        self.self_modifier.as_ref().is_some_and(|sm| sm.approve(id))
    }

    pub fn reject_proposal(&self, id: usize) -> bool {
        self.self_modifier.as_ref().is_some_and(|sm| sm.reject(id))
    }

    pub fn pending_proposals(&self) -> usize {
        self.self_modifier
            .as_ref()
            .map_or(0, |sm| sm.pending_count())
    }

    pub fn goal_count(&self) -> usize {
        self.goal_store.as_ref().map_or(0, |gs| gs.count())
    }

    pub fn active_goal_count(&self) -> usize {
        self.goal_store.as_ref().map_or(0, |gs| gs.active_count())
    }
}
