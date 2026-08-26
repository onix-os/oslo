// Turn Fig completion specs into carapace spec files oslo can read.
//
//   bun scripts/fig-to-spec.ts <fig-checkout>/src <out-dir> [--only name,name]
//
// Fig specs are TypeScript that *evaluates* to an object, so they are imported rather than parsed:
// a spec is a program, and the only thing that reads a program correctly is a runtime. bun runs
// TypeScript directly, which is why there is no build step here.
//
// # What cannot come across, and is counted rather than dropped in silence
//
// A Fig `generator` is a JavaScript function — it runs in Fig's own process, with Fig's API, and
// there is nothing on the other side of this conversion that could call it. The ones that carry a
// static `script` do come across, as a `$(…)` macro; the rest are counted and reported. Everything
// counted is printed at the end, so the shape of what was lost is a number rather than a surprise.

import { readdirSync, statSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { join, resolve, basename } from "node:path";

// ---------------------------------------------------------------- what a carapace spec looks like

type Values = string[];

/** A flag is a description, or a table when it has to say more than one thing. */
type Flag = string | { description: string; nargs?: number };

interface Command {
  name: string;
  aliases?: string[];
  description?: string;
  hidden?: boolean;
  parsing?: string;
  flags?: Record<string, Flag>;
  persistentflags?: Record<string, Flag>;
  exclusiveflags?: string[][];
  completion?: {
    flag?: Record<string, Values>;
    positional?: Values[];
    positionalany?: Values;
  };
  commands?: Command[];
}

// ---------------------------------------------------------------- counters, reported at the end

const lost = {
  generators: 0,
  generatorScripts: 0,
  generatorPostProcess: 0,
  templateHistory: 0,
  templateHelp: 0,
  exclusiveOn: 0,
  dependsOn: 0,
  argNames: 0,
  loadSpecInlined: 0,
  loadSpecDropped: 0,
  generateSpec: 0,
  commaInName: 0,
};

// ---------------------------------------------------------------- reading one Fig value

/** Fig writes a name as one string or as every spelling. */
function names(value: unknown): string[] {
  if (typeof value === "string") return [value];
  if (Array.isArray(value)) return value.filter((v) => typeof v === "string");
  return [];
}

function text(value: unknown): string | undefined {
  return typeof value === "string" && value.length > 0 ? value : undefined;
}

/** One line of a carapace value list: `value`, or `value\tdescription`. */
function value(name: string, description?: string): string {
  const clean = description?.replace(/\s+/g, " ").trim();
  return clean ? `${name}\t${clean}` : name;
}

/**
 * What one Fig argument completes to.
 *
 * The order matters: explicit suggestions first, then a generator's script, then the template.
 * A position that ends up with nothing at all answers `undefined`, and the caller leaves the
 * position undeclared rather than declaring an empty one — an empty declaration would suppress
 * oslo's own path completion, which is the better answer when a spec has nothing to say.
 */
function argValues(arg: any): Values | undefined {
  const out: Values = [];

  for (const suggestion of arg?.suggestions ?? []) {
    if (typeof suggestion === "string") {
      out.push(value(suggestion));
      continue;
    }
    for (const name of names(suggestion?.name)) {
      out.push(value(name, suggestion?.description));
    }
  }

  const generators = Array.isArray(arg?.generators)
    ? arg.generators
    : arg?.generators
      ? [arg.generators]
      : [];
  for (const generator of generators) {
    const script = generator?.script;
    // **A generator with a `postProcess` is its script AND that function**, and the function is
    // usually the half that matters: `docker ps --format "{{json .}}"` postProcessed to a name is
    // a list of containers, and the same script *without* it is one 400KB line of JSON offered as
    // a single insertable candidate. Half a generator is worse than none.
    if (typeof generator?.postProcess === "function") {
      lost.generatorPostProcess++;
      continue;
    }
    // Only a *static* script. A function `script` is Fig calling back into its own runtime with
    // the tokens typed so far, and there is nothing here that could stand in for that.
    if (typeof script === "string") {
      out.push(`$(${script})`);
      lost.generatorScripts++;
    } else if (Array.isArray(script) && script.every((s) => typeof s === "string")) {
      out.push(`$(${script.map(shellQuote).join(" ")})`);
      lost.generatorScripts++;
    } else {
      lost.generators++;
    }
  }

  for (const template of names(arg?.template)) {
    if (template === "filepaths") out.push("$files");
    else if (template === "folders") out.push("$directories");
    else if (template === "history") lost.templateHistory++;
    else if (template === "help") lost.templateHelp++;
  }

  // A named placeholder — `<URL>`, `<certificate[:password]>` — is documentation for a person and
  // there is no column in the dropdown that shows it without also inserting it.
  if (arg?.name && out.length === 0) lost.argNames++;

  return out.length > 0 ? out : undefined;
}

/** One word, quoted so a shell reads it back as itself. */
function shellQuote(word: string): string {
  return /^[A-Za-z0-9_./:=-]+$/.test(word) ? word : `'${word.replace(/'/g, `'\\''`)}'`;
}

// ---------------------------------------------------------------- one option

/**
 * The carapace key for a Fig option: every spelling, then the modifiers.
 *
 * `["-f", "--file"]` with an argument becomes `-f, --file=`, which is what carapace-spec writes and
 * what oslo's own flag parser reads.
 */
function flagKey(option: any): { key: string; nargs?: number } | undefined {
  // **A spelling containing a comma cannot be written at all.** The comma is the separator between
  // spellings, and the declaration syntax has no escape for one inside a name — so `ls`'s real
  // `-,` flag would be read back as a flag named `-`. Dropping it loses one flag; keeping it
  // invents a bogus one and loses the same flag anyway.
  const spellings = names(option?.name).filter((n) => {
    if (!n.startsWith("-")) return false;
    if (n.includes(",")) {
      lost.commaInName++;
      return false;
    }
    return true;
  });
  if (spellings.length === 0) return undefined;

  const args = Array.isArray(option?.args) ? option.args : option?.args ? [option.args] : [];
  const optional = args.length > 0 && args.every((a: any) => a?.isOptional);

  let key = spellings.join(", ");
  if (optional) key += "?";
  else if (args.length > 0) key += "=";
  if (option?.isRepeatable) key += "*";
  if (option?.isRequired) key += "!";
  if (option?.hidden) key += "&";

  // **How many words the argument is, when it is not one.** `=` alone means exactly one, so a
  // variadic argument (`git branch -d a b c`) and a two-word one (`git config --get-urlmatch
  // <section> <url>`) both need saying — otherwise the walk stops after the first word and counts
  // the rest as the command's positional arguments, which throws off every position after it.
  let nargs: number | undefined;
  if (args.length > 0) {
    if (args.some((a: any) => a?.isVariadic)) nargs = -1;
    else if (args.length > 1) nargs = args.length;
  }
  return { key, nargs };
}

/** The name `completion.flag` keys on: the longhand, without its dashes. */
function completionKey(option: any): string | undefined {
  const spellings = names(option?.name).filter((n) => n.startsWith("-"));
  const long = spellings.find((n) => n.startsWith("--")) ?? spellings[0];
  return long?.replace(/^-+/, "") || undefined;
}

// ---------------------------------------------------------------- one command, recursively

function command(spec: any, fallbackName?: string): Command | undefined {
  const spellings = names(spec?.name);
  const name = spellings[0] ?? fallbackName;
  if (!name) return undefined;

  const out: Command = { name };
  if (spellings.length > 1) out.aliases = spellings.slice(1);
  const description = text(spec?.description);
  if (description) out.description = description.replace(/\s+/g, " ").trim();
  if (spec?.hidden) out.hidden = true;

  // Fig says "the options must come first" where carapace says "flags stop at the first argument".
  if (spec?.parserDirectives?.optionsMustPrecedeArguments) out.parsing = "non-interspersed";

  const flags: Record<string, string> = {};
  const persistent: Record<string, string> = {};
  const flagValues: Record<string, Values> = {};
  const exclusive: string[][] = [];

  for (const option of spec?.options ?? []) {
    const declared = flagKey(option);
    if (!declared) continue;
    const { key, nargs } = declared;
    const into = option?.isPersistent ? persistent : flags;
    const description = (text(option?.description) ?? "").replace(/\s+/g, " ").trim();
    into[key] = nargs === undefined ? description : { description, nargs };

    const args = Array.isArray(option?.args) ? option.args : option?.args ? [option.args] : [];
    // carapace keys a flag's values by name and takes one list, so a multi-argument option's
    // *first* argument is the one that can be described. `nargs` would say how many there are;
    // this reader has no place to put it against a key that is already carrying modifiers.
    const values = args.length > 0 ? argValues(args[0]) : undefined;
    const named = completionKey(option);
    if (values && named) flagValues[named] = values;

    if (Array.isArray(option?.exclusiveOn) && option.exclusiveOn.length > 0) {
      const group = [named, ...option.exclusiveOn.map((n: string) => n.replace(/^-+/, ""))].filter(
        Boolean,
      ) as string[];
      if (group.length > 1) exclusive.push(group);
      lost.exclusiveOn++;
    }
    if (Array.isArray(option?.dependsOn) && option.dependsOn.length > 0) lost.dependsOn++;
  }

  if (Object.keys(flags).length > 0) out.flags = flags;
  if (Object.keys(persistent).length > 0) out.persistentflags = persistent;
  if (exclusive.length > 0) out.exclusiveflags = exclusive;

  // `args` is one argument or a list of them, and a variadic one answers for every position from
  // where it sits onwards.
  const args = Array.isArray(spec?.args) ? spec.args : spec?.args ? [spec.args] : [];
  const positional: Values[] = [];
  let positionalany: Values | undefined;
  for (const arg of args) {
    const values = argValues(arg);
    if (arg?.isVariadic) {
      positionalany = values;
      break;
    }
    positional.push(values ?? []);
  }
  // Trailing positions nobody could say anything about are not positions worth declaring.
  while (positional.length > 0 && positional[positional.length - 1].length === 0) positional.pop();

  const completion: Command["completion"] = {};
  if (Object.keys(flagValues).length > 0) completion.flag = flagValues;
  if (positional.length > 0) completion.positional = positional;
  if (positionalany && positionalany.length > 0) completion.positionalany = positionalany;
  if (Object.keys(completion).length > 0) out.completion = completion;

  const commands: Command[] = [];
  for (const sub of spec?.subcommands ?? []) {
    const child = command(sub);
    if (child) commands.push(child);
  }
  if (commands.length > 0) out.commands = commands;

  return out;
}

/**
 * Fig splits a large spec across files: `{ name: "compose", loadSpec: "docker-compose" }` means
 * "everything under this subcommand lives in docker-compose.ts". A converter that reads only the
 * literal tree drops those subtrees **entirely and silently** — `docker compose` becomes a leaf.
 *
 * The target is another file in the same corpus, so it is loaded and spliced in. A `loadSpec` that
 * is a *function* is Fig computing the tree from the tokens typed so far, which nothing here can
 * do; those are counted.
 */
async function inlineLoadSpec(node: any, source: string, seen: Set<string>): Promise<void> {
  for (const sub of node?.subcommands ?? []) {
    await inlineLoadSpec(sub, source, seen);

    const target = sub?.loadSpec;
    if (!target) continue;
    if (typeof target !== "string") {
      lost.loadSpecDropped++;
      continue;
    }
    // A cycle would otherwise be an unbounded splice: `a` loading `b` loading `a`.
    if (seen.has(target)) {
      lost.loadSpecDropped++;
      continue;
    }
    try {
      const module = await import(join(source, `${target}.ts`));
      const loaded = module.default ?? module.completionSpec;
      const resolved = typeof loaded === "function" ? await loaded("") : loaded;
      if (!resolved) {
        lost.loadSpecDropped++;
        continue;
      }
      const nested = new Set(seen).add(target);
      await inlineLoadSpec(resolved, source, nested);
      // The *contents*, not the command: the subcommand keeps the name it was reached by.
      sub.subcommands = [...(sub.subcommands ?? []), ...(resolved.subcommands ?? [])];
      sub.options = [...(sub.options ?? []), ...(resolved.options ?? [])];
      if (resolved.args && !sub.args) sub.args = resolved.args;
      lost.loadSpecInlined++;
    } catch {
      lost.loadSpecDropped++;
    }
  }
}

// ---------------------------------------------------------------- writing it out

/**
 * A scalar as this writer emits it.
 *
 * Everything that is not a bare word is double-quoted, because a description is arbitrary English
 * and a flag key is arbitrary punctuation. oslo's reader is a deliberate *subset* of YAML, so the
 * writer stays inside the same subset: no anchors, no tags, no plain scalars that could be read as
 * anything but text.
 */
function scalar(text: string): string {
  if (/^[A-Za-z0-9_][A-Za-z0-9_.\/-]*$/.test(text) && !/^(true|false|null|yes|no|on|off)$/i.test(text)) {
    return text;
  }
  const escaped = text
    .replace(/\\/g, "\\\\")
    .replace(/"/g, '\\"')
    .replace(/\t/g, "\\t")
    .replace(/\r/g, "\\r")
    .replace(/\n/g, "\\n");
  return `"${escaped}"`;
}

/** A `["a", "b"]` list, always in flow style: that is how carapace writes a value list. */
function flow(values: string[]): string {
  return `[${values.map(scalar).join(", ")}]`;
}

function emit(cmd: Command, indent: string, lines: string[]): void {
  lines.push(`${indent}name: ${scalar(cmd.name)}`);
  if (cmd.aliases?.length) lines.push(`${indent}aliases: ${flow(cmd.aliases)}`);
  if (cmd.description) lines.push(`${indent}description: ${scalar(cmd.description)}`);
  if (cmd.hidden) lines.push(`${indent}hidden: true`);
  if (cmd.parsing) lines.push(`${indent}parsing: ${cmd.parsing}`);

  for (const [key, table] of [
    ["flags", cmd.flags],
    ["persistentflags", cmd.persistentflags],
  ] as const) {
    if (!table) continue;
    lines.push(`${indent}${key}:`);
    for (const [flag, declared] of Object.entries(table)) {
      // A flag that has only a description is written as one; anything more takes the extended
      // notation, which is a flow mapping and stays on one line.
      const written =
        typeof declared === "string"
          ? scalar(declared)
          : `{description: ${scalar(declared.description)}, nargs: ${declared.nargs}}`;
      lines.push(`${indent}  ${scalar(flag)}: ${written}`);
    }
  }

  if (cmd.exclusiveflags?.length) {
    lines.push(`${indent}exclusiveflags:`);
    for (const group of cmd.exclusiveflags) lines.push(`${indent}  - ${flow(group)}`);
  }

  if (cmd.completion) {
    lines.push(`${indent}completion:`);
    if (cmd.completion.flag) {
      lines.push(`${indent}  flag:`);
      for (const [flag, values] of Object.entries(cmd.completion.flag)) {
        lines.push(`${indent}    ${scalar(flag)}: ${flow(values)}`);
      }
    }
    if (cmd.completion.positional) {
      lines.push(`${indent}  positional:`);
      for (const values of cmd.completion.positional) lines.push(`${indent}    - ${flow(values)}`);
    }
    if (cmd.completion.positionalany) {
      lines.push(`${indent}  positionalany: ${flow(cmd.completion.positionalany)}`);
    }
  }

  if (cmd.commands?.length) {
    lines.push(`${indent}commands:`);
    for (const child of cmd.commands) {
      const body: string[] = [];
      emit(child, "", body);
      lines.push(`${indent}  - ${body[0]}`);
      for (const line of body.slice(1)) lines.push(`${indent}    ${line}`);
    }
  }
}

// ---------------------------------------------------------------- the run

/** Every Fig spec in `dir`: `name.ts` is one, and `name/index.ts` is one too. */
function specFiles(dir: string): { command: string; path: string }[] {
  const found: { command: string; path: string }[] = [];
  for (const entry of readdirSync(dir)) {
    const path = join(dir, entry);
    if (statSync(path).isDirectory()) {
      const index = join(path, "index.ts");
      try {
        if (statSync(index).isFile()) found.push({ command: entry, path: index });
      } catch {
        // A directory of fragments with no index is not a spec of its own.
      }
      continue;
    }
    if (entry.endsWith(".ts") && !entry.endsWith(".d.ts")) {
      found.push({ command: entry.slice(0, -3), path });
    }
  }
  return found.sort((a, b) => a.command.localeCompare(b.command));
}

const [, , sourceArg, outArg, ...rest] = process.argv;
if (!sourceArg || !outArg) {
  console.error("usage: bun scripts/fig-to-spec.ts <fig>/src <out-dir> [--only a,b]");
  process.exit(2);
}
const source = resolve(sourceArg);
const out = resolve(outArg);
const onlyArg = rest.indexOf("--only");
const only =
  onlyArg >= 0 && rest[onlyArg + 1] ? new Set(rest[onlyArg + 1].split(",")) : undefined;

rmSync(out, { recursive: true, force: true });
mkdirSync(out, { recursive: true });

let written = 0;
let empty = 0;
const failed: { command: string; why: string }[] = [];

for (const { command: name, path } of specFiles(source)) {
  if (only && !only.has(name)) continue;
  try {
    const module = await import(path);
    const spec = module.default ?? module.completionSpec;
    // A `Fig.Spec` may be a function of the version, for a command whose shape changed.
    const resolved = typeof spec === "function" ? await spec("") : spec;
    if (typeof resolved?.generateSpec === "function") lost.generateSpec++;
    await inlineLoadSpec(resolved, source, new Set([name]));
    const converted = command(resolved, name);
    if (!converted) {
      failed.push({ command: name, why: "no name in the spec" });
      continue;
    }
    // **A spec with nothing in it is worse than no spec.** It is a file saying "this command has
    // no completions", which is what a spec built entirely out of `generateSpec` converts to —
    // and the reader would rather fall through to its own path completion than read that.
    if (
      !converted.flags &&
      !converted.persistentflags &&
      !converted.commands &&
      !converted.completion
    ) {
      empty++;
      continue;
    }
    // The file name is what oslo looks the spec up by, so it wins over whatever `name` says.
    converted.name = name;
    const lines: string[] = [
      `# Converted from Fig's ${basename(path)} by scripts/fig-to-spec.ts. Do not edit by hand.`,
    ];
    emit(converted, "", lines);
    writeFileSync(join(out, `${name}.yaml`), lines.join("\n") + "\n");
    written++;
  } catch (error) {
    failed.push({ command: name, why: String((error as Error)?.message ?? error).slice(0, 120) });
  }
}

console.log(`written  ${written}`);
console.log(`failed   ${failed.length}`);
console.log(`empty    ${empty}`);
for (const { command: name, why } of failed.slice(0, 40)) console.log(`  ${name}: ${why}`);
if (failed.length > 40) console.log(`  … and ${failed.length - 40} more`);
console.log(`\nnot carried across:`);
for (const [what, count] of Object.entries(lost)) console.log(`  ${what.padEnd(18)} ${count}`);
