use notist::{MemoryResources, Vault};
use notist_ssg::{Builder, SiteConfig};

#[test]
fn repeated_links_report_only_missing_anchors() {
    for references in [
        "[ok](./target.not#present) [bad](./target.not#absent)",
        "[bad](./target.not#absent) [ok](./target.not#present)",
    ] {
        let mut files = MemoryResources::new("/site");
        files.insert("content/index.not", references);
        files.insert("content/target.not", "@(id: \"present\")\n= Target");
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
