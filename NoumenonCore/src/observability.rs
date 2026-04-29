use anyhow::Result;

#[derive(Debug, Clone)]
pub struct MetricEvent {
    pub name: String,
    pub value: f64,
    pub labels: Vec<(String, String)>,
}

pub trait MetricsSink: Send + Sync {
    fn record(&self, event: MetricEvent);
    fn snapshot(&self) -> Vec<MetricEvent>;
}

#[derive(Debug, Default)]
pub struct InMemoryMetricsSink {
    events: std::sync::Mutex<Vec<MetricEvent>>,
}

impl MetricsSink for InMemoryMetricsSink {
    fn record(&self, event: MetricEvent) {
        if let Ok(mut g) = self.events.lock() {
            g.push(event);
        }
    }

    fn snapshot(&self) -> Vec<MetricEvent> {
        self.events.lock().map(|v| v.clone()).unwrap_or_default()
    }
}

pub struct AlertRule {
    pub metric: String,
    pub threshold: f64,
}

pub struct AlertEngine {
    pub rules: Vec<AlertRule>,
}

impl AlertEngine {
    pub fn evaluate(&self, sink: &dyn MetricsSink) -> Vec<String> {
        let events = sink.snapshot();
        let mut alerts = Vec::new();
        for r in &self.rules {
            for e in &events {
                if e.name == r.metric && e.value > r.threshold {
                    alerts.push(format!("alert:{}>{}", r.metric, r.threshold));
                }
            }
        }
        alerts
    }
}

pub struct ReportBuilder;

impl ReportBuilder {
    pub fn summarize(events: &[MetricEvent]) -> String {
        let count = events.len();
        let avg = if count == 0 {
            0.0
        } else {
            events.iter().map(|e| e.value).sum::<f64>() / count as f64
        };
        format!("events={} avg={:.3}", count, avg)
    }
}

pub struct SloTarget {
    pub metric: String,
    pub max_value: f64,
}

pub struct SloAdvisor;

impl SloAdvisor {
    pub fn advise(target: &SloTarget, sink: &dyn MetricsSink) -> Result<String> {
        let events = sink.snapshot();
        let relevant: Vec<&MetricEvent> =
            events.iter().filter(|e| e.name == target.metric).collect();
        if relevant.is_empty() {
            return Ok("no-data".to_string());
        }
        let avg = relevant.iter().map(|e| e.value).sum::<f64>() / relevant.len() as f64;
        if avg <= target.max_value {
            Ok("slo-healthy".to_string())
        } else {
            Ok(format!("slo-violation recommend-throttle avg={avg:.3}"))
        }
    }
}
