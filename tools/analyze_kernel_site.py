#!/usr/bin/env python3
"""Summarize clean latency separately from instrumented exclusive wall phases."""
import argparse
import json
from pathlib import Path
import statistics


def summarize(rows):
    normal = [r for r in rows if r["kind"] == "normal" and not r["initial_publication"]]
    result = {"clean_repeat": {field: {"median": statistics.median(r[field] for r in normal),
                                      "min": min(r[field] for r in normal),
                                      "max": max(r[field] for r in normal)}
                               for field in ["wall_s", "user_s", "system_s", "maxrss_kib"]},
              "initial": next(r for r in rows if r["initial_publication"]),
              "output_files": normal[0]["output_files"], "output_bytes": normal[0]["output_bytes"]}
    for kind in ["coarse", "detail"]:
        selected = [r for r in rows if r["kind"] == kind]
        result[kind] = {"wall_s": statistics.median(r["wall_s"] for r in selected),
                        "metrics": {name: {field: statistics.mean(r["metrics"][name][field] for r in selected)
                                           for field in selected[0]["metrics"][name]}
                                    for name in selected[0]["metrics"]},
                        "modules": selected[0]["modules"]}
        counts = {name: metric["calls"] for name, metric in selected[0]["modules"].items()}
        assert all({name: metric["calls"] for name, metric in r["modules"].items()} == counts for r in selected)
    t = lambda name: result["coarse"]["metrics"][name]["seconds"]
    stages = ["ssg.discover", "ssg.extension", "ssg.source_directories", "ssg.pipeline",
              "ssg.assets", "ssg.bodies", "ssg.shared_context", "ssg.templates", "ssg.finish"]
    drops = ["ssg.drop.values", "ssg.drop.dirs", "ssg.drop.components", "ssg.drop.by_source", "ssg.drop.engine"]
    partition = {name: t(name) for name in stages if name != "ssg.pipeline"}
    partition.update({name: t(name) for name in drops})
    partition.update({"analyze": t("pipeline.analyze"), "evaluate": t("pipeline.evaluate"),
                      "document_prepare_and_pipeline_other": t("ssg.pipeline")-t("pipeline.analyze")-t("pipeline.evaluate"),
                      "ssg_automatic_drop_and_other": t("ssg.total")-sum(t(n) for n in stages+drops),
                      "vault_drop": t("vault.drop"),
                      "host_setup_and_other": t("cli.assemble")-t("ssg.total")-t("vault.drop"),
                      "publish": t("cli.publish"), "artifact_drop": t("artifact.drop"),
                      "cli_other": t("cli.total")-t("cli.assemble")-t("cli.publish")-t("artifact.drop")})
    assert all(v >= -1e-7 for v in partition.values()), partition
    assert abs(sum(partition.values())-t("cli.total")) < 1e-7
    result["coarse_partition"] = {k: {"seconds": v, "percent": 100*v/t("cli.total")} for k,v in partition.items()}
    result["detail_self"] = sorted(({"name": name, **metric} for name,metric in result["detail"]["metrics"].items()),
                                    key=lambda r: r["self_seconds"], reverse=True)
    result["coarse_overhead_ratio"] = result["coarse"]["wall_s"]/result["clean_repeat"]["wall_s"]["median"]
    result["detail_overhead_ratio"] = result["detail"]["wall_s"]/result["clean_repeat"]["wall_s"]["median"]
    return result


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("out", type=Path)
    args = parser.parse_args()
    result = summarize(json.loads((args.out / "samples.json").read_text()))
    (args.out / "summary.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({"clean": result["clean_repeat"], "partition": result["coarse_partition"],
                      "probe_overhead": [result["coarse_overhead_ratio"], result["detail_overhead_ratio"]]}, indent=2))
