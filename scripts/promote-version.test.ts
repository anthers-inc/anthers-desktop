// SPDX-License-Identifier: Apache-2.0
//
// The version script's suite. The bump decision and the calver arithmetic are the two
// places a quiet mistake publishes a wrong number, so both are pinned here rather than
// trusted to a green release build: a release build succeeds against whatever version
// each tool reads, which is exactly how four fields drift apart unnoticed.
//
// ⚠️ Importing this module must never bump anything — `main` sits behind
// `import.meta.main`, and the platform repository learnt that the hard way when a test
// import executed the whole flow and silently wrote a version into the tree.

import { describe, expect, it } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { FIELDS, agreedVersion, bumpPlan, nextVersion, readField, writeField } from "./promote-version.ts";

const ROOT = join(import.meta.dir, "..");
const OCT = new Date(2026, 9, 28);

describe("nextVersion", () => {
	it("counts up inside a month", () => {
		expect(nextVersion("2026.10.0", OCT)).toBe("2026.10.1");
		expect(nextVersion("2026.10.37", OCT)).toBe("2026.10.38");
	});

	it("starts a new month at .0", () => {
		expect(nextVersion("2026.9.4", OCT)).toBe("2026.10.0");
		expect(nextVersion("2025.12.9", OCT)).toBe("2026.10.0");
	});

	it("refuses anything that is not calver", () => {
		expect(() => nextVersion("0.1.0", OCT)).toThrow(/not calver/);
		expect(() => nextVersion("v2026.10.0", OCT)).toThrow(/not calver/);
	});
});

describe("bumpPlan", () => {
	it("writes on a clean tree", () => {
		expect(bumpPlan("2026.10.0", "2026.10.0", undefined, OCT)).toEqual({
			action: "write",
			target: "2026.10.1",
		});
	});

	it("converges when a resumed promote already wrote the bump", () => {
		expect(bumpPlan("2026.10.0", "2026.10.1", undefined, OCT)).toEqual({
			action: "no-op",
			target: "2026.10.1",
		});
	});

	it("refuses a value no step wrote", () => {
		expect(bumpPlan("2026.10.0", "2026.10.7", undefined, OCT)).toEqual({
			action: "refuse",
			target: "2026.10.1",
			committed: "2026.10.0",
			working: "2026.10.7",
		});
	});

	it("lets a pin overwrite a stray value", () => {
		expect(bumpPlan("2026.10.0", "2026.10.7", "2026.10.3", OCT)).toEqual({
			action: "write",
			target: "2026.10.3",
		});
		expect(bumpPlan("2026.10.0", "2026.10.3", "2026.10.3", OCT)).toEqual({
			action: "no-op",
			target: "2026.10.3",
		});
	});

	// The one crossing of a scheme line: the fields carried 0.1.0 from the Studio era,
	// and a date cannot be computed from a semver value. The first calver release names
	// itself, and everything after it is arithmetic.
	it("starts the calver line from a semver HEAD when pinned", () => {
		expect(bumpPlan("0.1.0", "0.1.0", "2026.10.0", OCT)).toEqual({
			action: "write",
			target: "2026.10.0",
		});
	});
});

describe("the four fields", () => {
	for (const { path, pattern } of FIELDS) {
		it(`reads and rewrites ${path}`, () => {
			const source = readFileSync(join(ROOT, path), "utf8");
			const version = readField(source, pattern);
			expect(version).not.toBeNull();

			const rewritten = writeField(source, pattern, "2031.2.7");
			expect(readField(rewritten, pattern)).toBe("2031.2.7");
			// Only the version moves: the rest of the file is byte-identical.
			expect(rewritten.replace("2031.2.7", version as string)).toBe(source);
		});
	}

	// The checkout's own invariant, and the one `--check` guards in CI: four fields,
	// one calver version. A tree mid-bump is the only legitimate way this fails.
	it("agrees across all four in this checkout", () => {
		const read = (path: string) => readFileSync(join(ROOT, path), "utf8");
		expect(agreedVersion(read, "test")).toMatch(/^\d{4}\.\d{1,2}\.\d+$/);
	});
});
