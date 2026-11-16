//! What the graphics device holds, summed by resource label: where a frame's memory goes (the
//! scene's multisampled targets, the frame texture, the read-back buffer, the font atlas, the
//! model's buffers). The backend's allocator report feeds it (`adapters::gpu::memory`).

use serde_json::{Value, json};

/// The device's allocations summed by label, largest first.
#[derive(Debug, Default, PartialEq)]
pub struct GpuMemory {
    /// Bytes in live allocations.
    pub allocated: u64,
    /// Bytes in the memory blocks they are carved from (unused space included).
    pub reserved: u64,
    /// (label, bytes, allocations), largest first.
    pub by_label: Vec<(String, u64, u32)>,
}

impl GpuMemory {
    pub fn new(allocated: u64, reserved: u64, allocations: impl IntoIterator<Item = (String, u64)>) -> Self {
        let mut by_label: Vec<(String, u64, u32)> = Vec::new();
        for (label, size) in allocations {
            match by_label.iter_mut().find(|(l, _, _)| *l == label) {
                Some(entry) => {
                    entry.1 += size;
                    entry.2 += 1;
                }
                None => by_label.push((label, size, 1)),
            }
        }
        by_label.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        GpuMemory { allocated, reserved, by_label }
    }

    /// For `frames.json`: totals and the `top` largest labels, in MiB.
    pub fn to_json(&self, top: usize) -> Value {
        let labels: Vec<Value> = self.by_label.iter().take(top).map(|(l, b, n)| json!({"label": l, "mib": mib(*b), "count": n})).collect();
        json!({"allocatedMib": mib(self.allocated), "reservedMib": mib(self.reserved), "byLabel": labels})
    }

    /// One log line: totals and the `top` largest labels.
    pub fn line(&self, top: usize) -> String {
        let labels: Vec<String> = self.by_label.iter().take(top).map(|(l, b, n)| format!("{l} {:.1} MiB ×{n}", mib(*b))).collect();
        format!("allocated {:.1} MiB, reserved {:.1} MiB; {}", mib(self.allocated), mib(self.reserved), labels.join(", "))
    }
}

fn mib(bytes: u64) -> f64 {
    (bytes as f64 / f64::from(1 << 20) * 10.0).round() / 10.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sums_by_label_largest_first() {
        let m = GpuMemory::new(7 << 20, 8 << 20, [("a".into(), 1 << 20), ("b".into(), 4 << 20), ("a".into(), 2 << 20)]);
        assert_eq!(m.by_label, vec![("b".into(), 4 << 20, 1), ("a".into(), 3 << 20, 2)]);
        assert_eq!(m.to_json(1)["byLabel"][0]["label"], "b");
        assert!(m.line(2).starts_with("allocated 7.0 MiB, reserved 8.0 MiB; b 4.0 MiB ×1, a 3.0 MiB ×2"));
    }
}
