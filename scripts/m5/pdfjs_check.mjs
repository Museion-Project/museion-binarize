import assert from "node:assert/strict";
import { readFile, writeFile } from "node:fs/promises";

import { getDocument, version } from "pdfjs-dist/legacy/build/pdf.mjs";

if (process.argv.length !== 5) {
  console.error("usage: node pdfjs_check.mjs <pdf> <ground-truth.json> <result.json>");
  process.exit(2);
}

const [, , pdfPath, truthPath, resultPath] = process.argv;
const truth = JSON.parse(await readFile(truthPath, "utf8"));
const bytes = new Uint8Array(await readFile(pdfPath));
const loadingTask = getDocument({ data: bytes, disableWorker: true });
const document = await loadingTask.promise;

try {
  assert.equal(document.numPages, truth.page_count);

  const pageTexts = [];
  const rotations = [];
  for (let pageNumber = 1; pageNumber <= document.numPages; pageNumber += 1) {
    const page = await document.getPage(pageNumber);
    rotations.push(page.rotate);
    const content = await page.getTextContent();
    pageTexts.push(content.items.map((item) => item.str ?? "").join(" "));
  }
  assert.deepEqual(rotations, [0, 90, 180, 270]);
  for (const expected of truth.expected_text) {
    assert.ok(
      pageTexts.some((text) => text.includes(expected)),
      `PDF.js did not extract ${JSON.stringify(expected)}`,
    );
  }

  const outline = [];
  const visit = async (items, level) => {
    for (const item of items) {
      const destination =
        typeof item.dest === "string" ? await document.getDestination(item.dest) : item.dest;
      assert.ok(Array.isArray(destination), `outline ${JSON.stringify(item.title)} has no destination`);
      const target = destination[0];
      const pageIndex = Number.isInteger(target) ? target : await document.getPageIndex(target);
      assert.equal(destination[1]?.name, "XYZ");
      assert.ok(Number.isFinite(destination[2]));
      assert.ok(Number.isFinite(destination[3]));
      outline.push({
        title: item.title,
        level,
        page_index: pageIndex,
        destination: {
          type: destination[1].name,
          x: destination[2],
          y: destination[3],
        },
      });
      await visit(item.items ?? [], level + 1);
    }
  };
  await visit((await document.getOutline()) ?? [], 0);

  assert.equal(outline.length, truth.outline.length);
  for (const [index, expected] of truth.outline.entries()) {
    assert.deepEqual(
      {
        title: outline[index].title,
        level: outline[index].level,
        page_index: outline[index].page_index,
      },
      expected,
    );
  }

  const result = {
    schema: "mpdf-m5-pdfjs-result",
    schema_version: "0.1",
    pdfjs_version: version,
    page_count: document.numPages,
    rotations,
    unicode_text_matches: truth.expected_text.length,
    outline,
  };
  await writeFile(resultPath, `${JSON.stringify(result, null, 2)}\n`, {
    flag: "wx",
  });
} finally {
  await loadingTask.destroy();
}
