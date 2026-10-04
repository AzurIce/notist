import test from "node:test";
import assert from "node:assert/strict";
import { decodeValue, readParameter } from "../component-protocol.js";

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
