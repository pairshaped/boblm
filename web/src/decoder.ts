// Used only if a schema carries no limit; the generated contract always does.
const DEFAULT_MAX_BODY_BYTES = 1_048_576;
type ObjectValue = Readonly<Record<string, unknown>>;
function object(value: unknown): value is ObjectValue {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
function array_(value: unknown): readonly unknown[] {
  return Array.isArray(value) ? value : [];
}
function array(value: unknown): readonly unknown[] {
  if (!Array.isArray(value)) throw new Error("Invalid generated schema");
  return value;
}

function matches(
  schema: unknown,
  value: unknown,
  root: ObjectValue,
  depth: number,
): boolean {
  if (depth > 64) return false;
  if (schema === true) return json(value, depth);
  if (schema === false || !object(schema)) return false;
  if (Object.keys(schema).length === 0) return json(value, depth);
  if (typeof schema.$ref === "string") {
    if (!schema.$ref.startsWith("#/$defs/") || !object(root.$defs))
      throw new Error("Unsupported schema reference");
    return matches(root.$defs[schema.$ref.slice(8)], value, root, depth + 1);
  }
  if ("const" in schema && value !== schema.const) return false;
  if (Array.isArray(schema.enum) && !schema.enum.includes(value)) return false;
  if (
    schema.oneOf !== undefined &&
    array(schema.oneOf).filter((branch) =>
      matches(branch, value, root, depth + 1),
    ).length !== 1
  )
    return false;
  if (
    schema.anyOf !== undefined &&
    !array(schema.anyOf).some((branch) =>
      matches(branch, value, root, depth + 1),
    )
  )
    return false;
  if (
    schema.allOf !== undefined &&
    !array(schema.allOf).every((branch) =>
      matches(branch, value, root, depth + 1),
    )
  )
    return false;
  const kinds = Array.isArray(schema.type)
    ? schema.type
    : schema.type === undefined
      ? []
      : [schema.type];
  if (
    kinds.length > 0 &&
    !kinds.some((kind) => {
      switch (kind) {
        case "null":
          return value === null;
        case "boolean":
          return typeof value === "boolean";
        case "string":
          return typeof value === "string";
        case "number":
          return typeof value === "number" && Number.isFinite(value);
        case "integer":
          return typeof value === "number" && Number.isSafeInteger(value);
        case "array":
          return Array.isArray(value);
        case "object":
          return object(value);
        default:
          throw new Error("Unsupported generated schema type");
      }
    })
  )
    return false;
  if (typeof value === "string") {
    const length = Array.from(value).length;
    if (typeof schema.minLength === "number" && length < schema.minLength)
      return false;
    if (typeof schema.maxLength === "number" && length > schema.maxLength)
      return false;
    if (
      typeof schema.pattern === "string" &&
      !new RegExp(schema.pattern, "u").test(value)
    )
      return false;
  }
  if (typeof value === "number") {
    if (typeof schema.minimum === "number" && value < schema.minimum)
      return false;
    if (typeof schema.maximum === "number" && value > schema.maximum)
      return false;
  }
  if (Array.isArray(value)) {
    if (typeof schema.minItems === "number" && value.length < schema.minItems)
      return false;
    if (typeof schema.maxItems === "number" && value.length > schema.maxItems)
      return false;
    if (
      schema.items !== undefined &&
      !value.every((item) => matches(schema.items, item, root, depth + 1))
    )
      return false;
  }
  if (object(value)) {
    const properties = object(schema.properties) ? schema.properties : {};
    if (
      Array.isArray(schema.required) &&
      !schema.required.every(
        (key) => typeof key === "string" && Object.hasOwn(value, key),
      )
    )
      return false;
    for (const [key, field] of Object.entries(value)) {
      if (Object.hasOwn(properties, key)) {
        if (!matches(properties[key], field, root, depth + 1)) return false;
      } else if (schema.additionalProperties === false) return false;
      else if (
        object(schema.additionalProperties) &&
        !matches(schema.additionalProperties, field, root, depth + 1)
      )
        return false;
    }
  }
  return true;
}
function json(value: unknown, depth: number): boolean {
  if (depth > 64) return false;
  if (value === null || typeof value === "string" || typeof value === "boolean")
    return true;
  if (typeof value === "number") return Number.isFinite(value);
  if (Array.isArray(value)) return value.every((item) => json(item, depth + 1));
  return (
    object(value) && Object.values(value).every((item) => json(item, depth + 1))
  );
}
const describe = (value: unknown): string =>
  value === null
    ? "null"
    : Array.isArray(value)
      ? "an array"
      : typeof value === "object"
        ? "an object"
        : typeof value === "string"
          ? `the string ${JSON.stringify(value.length > 40 ? `${value.slice(0, 40)}...` : value)}`
          : `the ${typeof value} ${String(value)}`;
const quote = (key: string) => JSON.stringify(key);
/**
 * Best-effort sentence for the first thing wrong, once `matches` has said something is.
 * Follows the same schema; returns null when it cannot name a cause.
 */
function explain(
  schema: unknown,
  value: unknown,
  root: ObjectValue,
  path: string,
  depth: number,
): string | null {
  if (depth > 64 || !object(schema)) return null;
  if (typeof schema.$ref === "string" && object(root.$defs))
    return explain(
      root.$defs[schema.$ref.slice(8)],
      value,
      root,
      path,
      depth + 1,
    );
  const where = path || "the value";
  const kinds = Array.isArray(schema.type)
    ? schema.type
    : schema.type === undefined
      ? []
      : [schema.type];
  const names = kinds.filter((kind) => kind !== "null");
  if (kinds.length > 0 && !matches({ type: kinds }, value, root, depth + 1))
    return `${where} must be ${kinds.join(" or ")}, but it is ${describe(value)}`;
  if (Array.isArray(schema.enum) && !schema.enum.includes(value))
    return `${where} must be one of ${schema.enum.map((v) => JSON.stringify(v)).join(", ")}, but it is ${describe(value)}`;
  if (typeof value === "string") {
    const length = Array.from(value).length;
    if (typeof schema.maxLength === "number" && length > schema.maxLength)
      return `${where} is ${length} characters but may be at most ${schema.maxLength}`;
    if (typeof schema.minLength === "number" && length < schema.minLength)
      return `${where} is ${length} characters but needs at least ${schema.minLength}`;
    if (
      typeof schema.pattern === "string" &&
      !new RegExp(schema.pattern, "u").test(value)
    )
      return `${where} is ${describe(value)}, which is not in the expected format`;
  }
  if (typeof value === "number") {
    if (typeof schema.maximum === "number" && value > schema.maximum)
      return `${where} is ${value} but may be at most ${schema.maximum}`;
    if (typeof schema.minimum === "number" && value < schema.minimum)
      return `${where} is ${value} but must be at least ${schema.minimum}`;
  }
  if (Array.isArray(value)) {
    if (typeof schema.maxItems === "number" && value.length > schema.maxItems)
      return `${where} has ${value.length} items but may have at most ${schema.maxItems}`;
    if (schema.items !== undefined)
      for (const [index, item] of value.entries()) {
        const found = explain(
          schema.items,
          item,
          root,
          `${path}[${index}]`,
          depth + 1,
        );
        if (found && !matches(schema.items, item, root, depth + 1))
          return found;
      }
  }
  if (object(value)) {
    const properties = object(schema.properties) ? schema.properties : {};
    if (Array.isArray(schema.required))
      for (const key of schema.required)
        if (typeof key === "string" && !Object.hasOwn(value, key))
          return `${where} is missing the required field ${quote(key)}`;
    for (const [key, field] of Object.entries(value)) {
      const child = path ? `${path}.${key}` : `field ${quote(key)}`;
      if (Object.hasOwn(properties, key)) {
        if (!matches(properties[key], field, root, depth + 1))
          return (
            explain(properties[key], field, root, child, depth + 1) ??
            `${child} is not valid`
          );
      } else if (schema.additionalProperties === false)
        return `${where} has a field ${quote(key)} that is not allowed here`;
    }
  }
  const branches = array_(schema.oneOf ?? schema.anyOf);
  if (branches.length) {
    // Tagged unions: find the branch the value claims by its "kind" (or "type") tag.
    const resolved = branches.map((branch) =>
      object(branch) && typeof branch.$ref === "string" && object(root.$defs)
        ? root.$defs[branch.$ref.slice(8)]
        : branch,
    );
    for (const tag of ["kind", "type"]) {
      const tags = resolved.map((branch) =>
        object(branch) &&
        object(branch.properties) &&
        object(branch.properties[tag])
          ? (branch.properties[tag] as ObjectValue).const
          : undefined,
      );
      if (tags.every((t) => typeof t === "string")) {
        const claimed = object(value) ? value[tag] : undefined;
        const index = (tags as unknown[]).indexOf(claimed);
        if (index >= 0)
          return explain(resolved[index], value, root, path, depth + 1);
        return `${path ? `${path}.${tag}` : `field ${quote(tag)}`} must be one of ${tags.map((t) => JSON.stringify(t)).join(", ")}, but it is ${describe(claimed)}`;
      }
    }
    return `${where} does not match any of the shapes allowed here (${describe(value)})`;
  }
  return names.length ? `${where} is not valid` : null;
}
export function decode(root: unknown, name: string, value: unknown): unknown {
  const serialized = JSON.stringify(value);
  if (
    serialized === undefined ||
    new TextEncoder().encode(serialized).byteLength >
      (object(root) && typeof root["x-maxBodyBytes"] === "number"
        ? root["x-maxBodyBytes"]
        : DEFAULT_MAX_BODY_BYTES)
  )
    throw new Error(
      "This message is larger than the 1 MiB limit for anything sent or received.",
    );
  if (!object(root) || !object(root.$defs) || !(name in root.$defs))
    throw new Error("Unknown generated schema");
  if (!matches(root.$defs[name], value, root, 0)) {
    const problem = explain(root.$defs[name], value, root, "", 0);
    throw new Error(
      problem
        ? `This ${name} is not valid: ${problem}.`
        : `This ${name} does not match the expected format.`,
    );
  }
  return value;
}
