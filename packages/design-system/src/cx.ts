/** Joins truthy class names. */
export function cx(...parts: (string | false | null | undefined)[]): string {
  return parts.filter(Boolean).join(" ");
}

let counter = 0;
/** Deterministic-enough id generator for non-SSR use (useId is preferred inside components). */
export function uniqueId(prefix = "co"): string {
  counter += 1;
  return `${prefix}-${counter}`;
}
