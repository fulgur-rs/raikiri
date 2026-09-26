import tempfile
import unittest
from pathlib import Path

from wptrunner_raikiri import (
    RaikiriPrintRefTestExecutor,
    RaikiriRefTestExecutor,
    _ordered_page_paths,
    _print_page_size,
    _select_page_paths,
    get_product,
)


class PrintProductTests(unittest.TestCase):
    def test_print_page_size_uses_css_pixels_per_inch(self):
        self.assertEqual(_print_page_size((5 * 2.54, 3 * 2.54)), (480, 288))

    def test_ordered_page_paths_returns_contiguous_numeric_order(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            second = directory / "page-0002.png"
            first = directory / "page-0001.png"
            second.write_bytes(b"second")
            first.write_bytes(b"first")

            self.assertEqual(_ordered_page_paths(directory), [first, second])

    def test_ordered_page_paths_rejects_empty_output(self):
        with tempfile.TemporaryDirectory() as temporary:
            with self.assertRaises(ValueError):
                _ordered_page_paths(Path(temporary))

    def test_ordered_page_paths_rejects_a_gap(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            (directory / "page-0002.png").write_bytes(b"data")
            with self.assertRaises(ValueError):
                _ordered_page_paths(directory)

    def test_ordered_page_paths_rejects_unexpected_files(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            (directory / "page-0001.png").write_bytes(b"data")
            (directory / "other.txt").write_bytes(b"data")
            with self.assertRaises(ValueError):
                _ordered_page_paths(directory)

    def test_selects_requested_print_pages(self):
        paths = [Path(f"page-{index:04}.png") for index in range(1, 5)]

        self.assertEqual(_select_page_paths(paths, [[2, 3]]), paths[1:3])

    def test_product_registers_screen_and_print_reftest_executors(self):
        product = get_product()

        self.assertIs(product.executor_classes["reftest"], RaikiriRefTestExecutor)
        self.assertIs(
            product.executor_classes["print-reftest"], RaikiriPrintRefTestExecutor
        )
        self.assertNotIn("testharness", product.executor_classes)


if __name__ == "__main__":
    unittest.main()
