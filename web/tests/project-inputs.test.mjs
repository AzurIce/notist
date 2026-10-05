import assert from "node:assert/strict";
import test from "node:test";
import { loadInputs } from "../project-inputs.js";

test("resource preparation includes root development dependencies, deduplicates public graphs and preserves Unicode", async () => {
  const manifests = {
    root: { package: { name: "root" }, dependencies: [{ name: "a", path: "../a" }], dev_dependencies: [{ name: "d", path: "../d" }] },
    a: { package: { name: "a" }, dependencies: [{ name: "shared", path: "../shared" }], dev_dependencies: [{ name: "private", path: "../private" }] },
    d: { package: { name: "d" }, dependencies: [{ name: "shared", path: "../shared" }], dev_dependencies: [] },
    shared: { package: { name: "shared" }, dependencies: [], dev_dependencies: [] },
  };
  const requests = [];
  const fetcher = async (url, options) => {
    url = new URL(url);
    const path = decodeURIComponent(url.pathname);
    requests.push([path, options?.method ?? "GET"]);
    const name = path.split("/")[1];
    if (path.endsWith("Notist.toml")) return new Response(name);
    if (path.endsWith("lib.notc")) return new Response('fn 图(text: String = "α<&") -> Content;');
    return new Response(null, { status: path === "/a/components/图.js" ? 200 : 404 });
  };
  const inputs = await loadInputs(new URL("https://host.test/root/Notist.toml"), {
    configuration: source => JSON.stringify(manifests[source]),
    describe_prepared: source => {
      const inputs = JSON.parse(source);
      assert.equal(inputs.root, "/root/");
      assert.equal(inputs.config, "/root/Notist.toml");
      assert.deepEqual(Object.keys(inputs.files).sort(), Object.keys(manifests).flatMap(name => [`/${name}/Notist.toml`, `/${name}/lib.notc`]).sort());
      return JSON.stringify({ functions: [{ package: "a", root: "/a", name: "图", entries: ["components/图.js", "components/图/index.js"] }] });
    },
  }, fetcher);
  assert.equal(new TextDecoder().decode(Uint8Array.from(inputs.files["/a/lib.notc"])), 'fn 图(text: String = "α<&") -> Content;');
  assert.equal(requests.filter(([path]) => path === "/shared/Notist.toml").length, 1);
  assert.equal(requests.filter(([path]) => path.startsWith("/private/")).length, 0);
  assert.equal(inputs.module_urls["/a/components/图.js"], "https://host.test/a/components/%E5%9B%BE.js");
});

test("missing dependency resources reach Rust's diagnostic assembly", async () => {
  const config = { package: null, dependencies: [{ name: "missing", path: "missing" }], dev_dependencies: [] };
  await assert.rejects(loadInputs(new URL("https://host.test/Notist.toml"), {
    configuration: () => JSON.stringify(config),
    describe_prepared: source => {
      assert.deepEqual(Object.keys(JSON.parse(source).files), ["/Notist.toml"]);
      return JSON.stringify({ diagnostics: [{ path: "/Notist.toml", message: "cannot load /missing/Notist.toml" }] });
    },
  }, async url => new Response(url.pathname === "/Notist.toml" ? "config" : null, { status: url.pathname === "/Notist.toml" ? 200 : 404 })), /cannot load/);
});

test("resource discovery terminates cycles and leaves semantic rejection to Rust", async () => {
  const requests = [];
  await assert.rejects(loadInputs(new URL("https://host.test/root/Notist.toml"), {
    configuration: () => JSON.stringify({ package: { name: "root" }, dependencies: [{ name: "root", path: "." }], dev_dependencies: [] }),
    describe_prepared: () => JSON.stringify({ diagnostics: [{ path: "/root/Notist.toml", message: "dependency cycle" }] }),
  }, async url => { requests.push(url.pathname); return new Response("source"); }), /dependency cycle/);
  assert.deepEqual(requests, ["/root/Notist.toml", "/root/lib.notc"]);
});
