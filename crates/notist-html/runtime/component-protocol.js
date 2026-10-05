// Notist component protocol v1. Integers use BigInt; ordered dictionaries use Map.
export function encodeName(name) {
  let encoded = Array.from(name, c => /[a-z0-9-]/.test(c) ? c : `u${c.codePointAt(0).toString(16)}x`).join("");
  if (!/^[a-z]/.test(encoded)) encoded = "x" + encoded;
  return encoded;
}
export function decodeValue(value) {
  const [type, payload] = value;
  switch (type) {
    case "unit": return null;
    case "bool": case "string": return payload;
    case "int": return BigInt(payload);
    case "float": {
      const bytes = new DataView(new ArrayBuffer(8));
      bytes.setBigUint64(0, BigInt("0x" + payload));
      return bytes.getFloat64(0);
    }
    case "array": return payload.map(decodeValue);
    case "dict": return new Map(payload.map(([key, value]) => [key, decodeValue(value)]));
    default: throw new Error(`Unknown Notist value type: ${type}`);
  }
}
export function readParameter(element, name, type) {
  if (element.getAttribute("notist-protocol") !== "1") throw new Error("Unsupported Notist component protocol");
  const value = element.getAttribute("notist-" + encodeName(name));
  if (value === null) return undefined;
  switch (type) {
    case "String": return value;
    case "Unit": return null;
    case "Bool": if (value !== "true" && value !== "false") throw new Error("Invalid Bool"); return value === "true";
    case "Int": return BigInt(value);
    case "Float": return Number(value);
    case "Array": case "Dict": return decodeValue(JSON.parse(value));
    default: throw new Error(`Unknown Notist parameter type: ${type}`);
  }
}
export async function registerComponents(components) {
  const seen = new Map();
  for (const component of components) {
    if (typeof component.module !== "string" || !component.module) throw new Error(`No published module URL for Notist component: ${component.tag}`);
    const previous = seen.get(component.tag);
    if (previous && previous !== component.module) throw new Error(`Conflicting Notist component: ${component.tag}`);
    if (previous) continue;
    seen.set(component.tag, component.module);
    const { default: implementation } = await import(component.module);
    const existing = customElements.get(component.tag);
    if (existing && existing !== implementation) throw new Error(`Conflicting Notist component: ${component.tag}`);
    if (!existing) customElements.define(component.tag, implementation);
  }
}
