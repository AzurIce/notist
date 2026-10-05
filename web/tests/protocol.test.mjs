import test from "node:test";
import assert from "node:assert/strict";
import { decodeValue, readParameter, registerComponents } from "../component-protocol.js";

test("collection protocol retains integer precision, float bits and dictionary order", () => {
  const decoded = decodeValue(["dict", [
    ["second", ["int", "9223372036854775807"]],
    ["first", ["float", "8000000000000000"]],
    ["values", ["array", [["unit"], ["bool", false], ["string", ""]]]],
  ]]);
  assert.deepEqual([...decoded.keys()], ["second", "first", "values"]);
  assert.equal(decoded.get("second"), 9223372036854775807n);
  assert.ok(Object.is(decoded.get("first"), -0));
  assert.deepEqual(decoded.get("values"), [null, false, ""]);
});
test("missing, empty and false scalar parameters remain distinct", () => {
  const fields = new Map([["notist-protocol", "1"], ["notist-label", ""], ["notist-open", "false"], ["notist-count", "-9223372036854775808"], ["notist-ratio", "Infinity"]]);
  const element = { getAttribute: name => fields.get(name) ?? null };
  assert.equal(readParameter(element, "label", "String"), "");
  assert.equal(readParameter(element, "open", "Bool"), false);
  assert.equal(readParameter(element, "missing", "String"), undefined);
  assert.equal(readParameter(element, "count", "Int"), -9223372036854775808n);
  assert.equal(readParameter(element, "ratio", "Float"), Infinity);
});

test("distributed runtime registers once and rejects missing URLs or conflicting implementations", async () => {
  const previous = globalThis.customElements;
  const constructors = new Map();
  let registrations = 0;
  globalThis.customElements = {
    get: tag => constructors.get(tag),
    define: (tag, implementation) => { registrations++; constructors.set(tag, implementation); },
  };
  try {
    const first = { tag: "test-widget", module: "data:text/javascript,export default class First {}" };
    await registerComponents([first, first]);
    await registerComponents([first]);
    assert.equal(registrations, 1);
    const second = { ...first, module: "data:text/javascript,export default class Second {}" };
    await assert.rejects(registerComponents([second]), /Conflicting Notist component/);
    await assert.rejects(registerComponents([first, second]), /Conflicting Notist component/);
    await assert.rejects(registerComponents([{ tag: "missing-widget", module: null }]), /No published module URL/);
  } finally { globalThis.customElements = previous; }
});
