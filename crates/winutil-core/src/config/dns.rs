//! `dns.json`: DNS providers the DNS tab can switch to.

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DnsProvider {
    /// Only two providers take part in the benchmark.
    #[serde(default)]
    pub benchmark_eligible: bool,
    pub primary: String,
    pub secondary: String,
    pub primary6: String,
    pub secondary6: String,
    pub doh_template: String,
}
