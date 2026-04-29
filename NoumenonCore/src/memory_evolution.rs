use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryNote {
    pub id: String,
    pub content: String,
    pub context: String,
    pub tags: Vec<String>,
    pub links: Vec<String>,
}

pub trait MemoryStore: Send + Sync {
    fn upsert(&mut self, note: MemoryNote);
    fn get(&self, id: &str) -> Option<MemoryNote>;
    fn by_tag(&self, tag: &str) -> Vec<MemoryNote>;
    fn all(&self) -> Vec<MemoryNote>;
}

#[derive(Debug, Default)]
pub struct InMemoryMemoryStore {
    notes: HashMap<String, MemoryNote>,
}

impl MemoryStore for InMemoryMemoryStore {
    fn upsert(&mut self, note: MemoryNote) {
        self.notes.insert(note.id.clone(), note);
    }

    fn get(&self, id: &str) -> Option<MemoryNote> {
        self.notes.get(id).cloned()
    }

    fn by_tag(&self, tag: &str) -> Vec<MemoryNote> {
        self.notes
            .values()
            .filter(|n| n.tags.iter().any(|t| t == tag))
            .cloned()
            .collect()
    }

    fn all(&self) -> Vec<MemoryNote> {
        self.notes.values().cloned().collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvolutionAction {
    Add,
    Update,
    Merge,
}

#[derive(Debug, Clone)]
pub struct EvolutionDecision {
    pub action: EvolutionAction,
    pub target_id: Option<String>,
    pub evolved: MemoryNote,
    pub reasoning: String,
}

pub struct MemoryEvolutionEngine;

impl MemoryEvolutionEngine {
    pub fn evolve(candidate: MemoryNote, neighbors: Vec<MemoryNote>) -> EvolutionDecision {
        if neighbors.is_empty() {
            return EvolutionDecision {
                action: EvolutionAction::Add,
                target_id: None,
                evolved: candidate,
                reasoning: "no-neighbor-add".to_string(),
            };
        }

        let best = neighbors
            .iter()
            .max_by_key(|n| token_overlap(&candidate.content, &n.content))
            .cloned();

        if let Some(target) = best {
            let overlap = token_overlap(&candidate.content, &target.content);
            if overlap >= 3 {
                let merged = merge_notes(candidate, target.clone());
                return EvolutionDecision {
                    action: EvolutionAction::Merge,
                    target_id: Some(target.id),
                    evolved: merged,
                    reasoning: format!("merge-overlap={overlap}"),
                };
            }

            let mut updated = target.clone();
            if !candidate.content.is_empty() {
                updated.content = format!("{}\n{}", target.content, candidate.content);
            }
            let mut tag_set: HashSet<String> = target.tags.into_iter().collect();
            for t in candidate.tags {
                tag_set.insert(t);
            }
            updated.tags = tag_set.into_iter().collect();
            return EvolutionDecision {
                action: EvolutionAction::Update,
                target_id: Some(updated.id.clone()),
                evolved: updated,
                reasoning: "update-nearest-neighbor".to_string(),
            };
        }

        EvolutionDecision {
            action: EvolutionAction::Add,
            target_id: None,
            evolved: candidate,
            reasoning: "fallback-add".to_string(),
        }
    }
}

fn token_overlap(a: &str, b: &str) -> usize {
    let sa: HashSet<&str> = a.split_whitespace().collect();
    let sb: HashSet<&str> = b.split_whitespace().collect();
    sa.intersection(&sb).count()
}

fn merge_notes(left: MemoryNote, right: MemoryNote) -> MemoryNote {
    let mut tag_set: HashSet<String> = left.tags.into_iter().collect();
    for t in right.tags {
        tag_set.insert(t);
    }
    let mut link_set: HashSet<String> = left.links.into_iter().collect();
    link_set.insert(right.id.clone());

    MemoryNote {
        id: right.id,
        content: format!("{}\n{}", right.content, left.content),
        context: if right.context.is_empty() {
            left.context
        } else {
            right.context
        },
        tags: tag_set.into_iter().collect(),
        links: link_set.into_iter().collect(),
    }
}
