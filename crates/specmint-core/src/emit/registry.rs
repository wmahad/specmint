//! Topological emitter registry (Kubb plugin sort analogue).

use std::collections::{HashMap, HashSet, VecDeque};

use crate::emit::factories::FactoriesEmitter;
use crate::emit::msw::MswEmitter;
use crate::emit::types::TypesEmitter;
use crate::emit::zod::ZodEmitter;
use crate::emit::{EmitCtx, Emitter, OutputFile};
use crate::error::{Error, Result};

pub struct EmitterRegistry {
    emitters: Vec<Box<dyn Emitter>>,
}

impl EmitterRegistry {
    pub fn builtin() -> Self {
        Self {
            emitters: vec![
                Box::new(TypesEmitter),
                Box::new(FactoriesEmitter),
                Box::new(MswEmitter),
                Box::new(ZodEmitter),
            ],
        }
    }

    pub fn run(&self, ctx: &EmitCtx<'_>) -> Result<Vec<OutputFile>> {
        let ordered = self.topo_order()?;
        let mut files = Vec::new();
        for emitter in ordered {
            if !emitter.enabled(ctx) {
                continue;
            }
            tracing::info!(emitter = emitter.id(), "running emitter");
            files.extend(emitter.generate(ctx)?);
        }
        Ok(files)
    }

    fn topo_order(&self) -> Result<Vec<&dyn Emitter>> {
        let id_to_idx: HashMap<&str, usize> = self
            .emitters
            .iter()
            .enumerate()
            .map(|(i, e)| (e.id(), i))
            .collect();

        let mut indegree = vec![0usize; self.emitters.len()];
        let mut adj: Vec<Vec<usize>> = vec![vec![]; self.emitters.len()];

        for (i, emitter) in self.emitters.iter().enumerate() {
            for dep in emitter.dependencies() {
                let Some(&dep_idx) = id_to_idx.get(dep) else {
                    return Err(Error::emit(
                        emitter.id(),
                        format!("unknown dependency `{dep}`"),
                    ));
                };
                adj[dep_idx].push(i);
                indegree[i] += 1;
            }
        }

        let mut queue: VecDeque<usize> = indegree
            .iter()
            .enumerate()
            .filter_map(|(i, d)| (*d == 0).then_some(i))
            .collect();
        let mut ordered = Vec::new();
        let mut seen = HashSet::new();

        while let Some(i) = queue.pop_front() {
            if !seen.insert(i) {
                continue;
            }
            ordered.push(self.emitters[i].as_ref());
            for &next in &adj[i] {
                indegree[next] -= 1;
                if indegree[next] == 0 {
                    queue.push_back(next);
                }
            }
        }

        if ordered.len() != self.emitters.len() {
            return Err(Error::emit(
                "registry",
                "cyclic emitter dependencies".to_string(),
            ));
        }

        Ok(ordered)
    }
}
