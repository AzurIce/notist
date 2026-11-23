use notist::{MemoryResources, Vault};
use notist_ssg::{Builder, SiteConfig};

#[test]
fn repeated_links_report_only_missing_anchors() {
    for references in [
        "[ok](./target.md#present) [bad](./target.md#absent)",
        "[bad](./target.md#absent) [ok](./target.md#present)",
    ] {
        let mut files = MemoryResources::new("/site");
        files.insert("content/index.md", references);
        files.insert("content/target.md", "@(id: \"present\")\n# Target");
        let site = Builder::default()
            .build(&mut Vault::new(files), &SiteConfig::default())
            .unwrap();
        assert_eq!(site.warnings.len(), 1, "{:?}", site.warnings);
        assert!(
            site.warnings[0]
                .diagnostic
                .message
                .contains("missing anchor `absent`"),
            "{:?}",
            site.warnings
        );
    }
}
