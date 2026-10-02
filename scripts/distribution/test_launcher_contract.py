"""Cross-language inventory contract parity, no child process or OCR."""
import re
import unittest
from pathlib import Path
from scripts.ocr.app_mvp_bridge.runtime_contract import SOURCE_FILES, INITIALIZERS, BUILD_METADATA


class LauncherContractTests(unittest.TestCase):
    def test_rust_whitelist_matches_python_producer_and_consumer(self):
        path=Path(__file__).resolve().parents[2]/'apps/desktop/src-tauri/src/local_runtime.rs'
        source=path.read_text()
        for key,expected in [('SOURCE_FILES',set(SOURCE_FILES)),('INITIALIZERS',INITIALIZERS),('METADATA',BUILD_METADATA)]:
            array=re.search(r'\b'+key+r':\s*&\[&str\]\s*=\s*&\[(.*?)\];',source,re.S)
            self.assertIsNotNone(array,key)
            actual=re.findall(r'"([^"\n]+)"',array.group(1))
            self.assertEqual(len(actual),len(set(actual)))
            self.assertEqual(set(actual),expected)


if __name__=='__main__':unittest.main()
