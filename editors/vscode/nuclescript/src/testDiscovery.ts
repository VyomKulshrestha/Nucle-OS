export interface DiscoveredTest {
  name: string;
  startOffset: number;
  endOffset: number;
}

/** Discover top-level `test "name" { ... }` declarations without treating
 * comments, strings, or nested declarations as tests. This is deliberately
 * a small lexer, not a second NucleScript parser: the CLI remains the source
 * of truth for whether the file compiles and for every test result. */
export function discoverTests(source: string): DiscoveredTest[] {
  const tests: DiscoveredTest[] = [];
  let depth = 0;
  let index = 0;

  while (index < source.length) {
    if (source[index] === "/" && source[index + 1] === "/") {
      index = skipLineComment(source, index);
      continue;
    }
    if (source[index] === '"') {
      index = readString(source, index).endOffset;
      continue;
    }
    if (source[index] === "{") {
      depth += 1;
      index += 1;
      continue;
    }
    if (source[index] === "}") {
      depth = Math.max(0, depth - 1);
      index += 1;
      continue;
    }
    if (!isIdentifierStart(source[index])) {
      index += 1;
      continue;
    }

    const startOffset = index;
    index += 1;
    while (index < source.length && isIdentifierContinue(source[index])) {
      index += 1;
    }
    const word = source.slice(startOffset, index);
    if (depth !== 0 || word.toLowerCase() !== "test") {
      continue;
    }

    const nameStart = skipTrivia(source, index);
    if (source[nameStart] !== '"') {
      continue;
    }
    const parsed = readString(source, nameStart);
    if (!parsed.terminated) {
      continue;
    }
    tests.push({ name: parsed.value, startOffset, endOffset: parsed.endOffset });
    index = parsed.endOffset;
  }

  return tests;
}

function skipTrivia(source: string, start: number): number {
  let index = start;
  while (index < source.length) {
    if (/\s/.test(source[index])) {
      index += 1;
    } else if (source[index] === "/" && source[index + 1] === "/") {
      index = skipLineComment(source, index);
    } else {
      break;
    }
  }
  return index;
}

function skipLineComment(source: string, start: number): number {
  const newline = source.indexOf("\n", start + 2);
  return newline === -1 ? source.length : newline + 1;
}

function readString(
  source: string,
  start: number,
): { value: string; endOffset: number; terminated: boolean } {
  let index = start + 1;
  let value = "";
  while (index < source.length) {
    const character = source[index];
    if (character === '"') {
      return { value, endOffset: index + 1, terminated: true };
    }
    if (character === "\\" && index + 1 < source.length) {
      const escaped = source[index + 1];
      value += escaped === "n" ? "\n" : escaped === "r" ? "\r" : escaped === "t" ? "\t" : escaped;
      index += 2;
      continue;
    }
    value += character;
    index += 1;
  }
  return { value, endOffset: source.length, terminated: false };
}

function isIdentifierStart(character: string | undefined): boolean {
  return character !== undefined && /[A-Za-z_]/.test(character);
}

function isIdentifierContinue(character: string | undefined): boolean {
  return character !== undefined && /[A-Za-z0-9_-]/.test(character);
}
