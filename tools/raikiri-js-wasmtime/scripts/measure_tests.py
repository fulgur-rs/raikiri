from pathlib import Path
import subprocess
import tempfile
import unittest


class MeasurementTests(unittest.TestCase):
    def failed_measurement(self, script):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            logs = root / 'target/integration-logs'
            for backend in ['native', 'wasmtime']:
                bins = logs / f'{backend}-bins'
                bins.mkdir(parents=True)
                for name in ['run-parsing-invalid', 'run-css-text-i18n']:
                    executable = bins / name
                    executable.write_text('#!/bin/sh\n' + script + '\n')
                    executable.chmod(0o755)
            stale = logs / 'native-parsing.json'
            stale.write_text('[{"stale":true}]')
            process = subprocess.run(['python3', str(Path(__file__).with_name('measure.py')), str(root)], capture_output=True, text=True)
            self.assertNotEqual(process.returncode, 0, process.stdout)
            self.assertFalse(stale.exists())
            return process.stderr

    def test_failed_run_removes_old_results_and_rejects_signal_exit(self):
        self.assertIn('exit', self.failed_measurement('kill -TERM $$'))

    def test_run_without_fresh_json_is_rejected(self):
        self.assertIn('results', self.failed_measurement('exit 0'))


if __name__ == '__main__':
    unittest.main()
