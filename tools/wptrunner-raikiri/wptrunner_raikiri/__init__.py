"""Connectionless upstream wptrunner product for Raikiri reftests."""

from __future__ import annotations

import base64
import contextlib
import os
import re
import shutil
import sys
import tempfile
import uuid
from pathlib import Path

from mozprocess import ProcessHandler
from tools.serve.serve import make_hosts_file
from wptrunner.browsers.base import (
    ExecutorBrowser,
    NullBrowser,
    get_timeout_multiplier,
    require_arg,
)
from wptrunner.executors import executor_kwargs as base_executor_kwargs
from wptrunner.executors.base import (
    RefTestExecutor,
    RefTestImplementation,
    get_pages,
    reftest_result_converter,
)
from wptrunner.products import Product


def check_args(**kwargs):
    require_arg(kwargs, "binary")


def browser_kwargs(logger, test_type, run_info_data, config, subsuite, **kwargs):
    del logger, test_type, run_info_data, config
    binary_args = list(kwargs.get("binary_args") or [])
    binary_args.extend(subsuite.config.get("binary_args", []))
    return {"binary": kwargs["binary"], "binary_args": binary_args}


def executor_kwargs(
    logger, test_type, test_environment, run_info_data, subsuite, **kwargs
):
    del logger
    return base_executor_kwargs(
        test_type,
        test_environment,
        run_info_data,
        subsuite,
        **kwargs,
    )


class RaikiriBrowser(NullBrowser):
    def __init__(self, logger, binary, binary_args=None, **kwargs):
        super().__init__(logger, **kwargs)
        self.binary = binary
        self.binary_args = binary_args or []

    def executor_browser(self):
        return ExecutorBrowser, {
            "binary": self.binary,
            "binary_args": self.binary_args,
        }


def _print_page_size(viewport_size):
    width_cm, height_cm = viewport_size
    pixels_per_cm = 96 / 2.54
    return round(width_cm * pixels_per_cm), round(height_cm * pixels_per_cm)


def _ordered_page_paths(directory):
    directory = Path(directory)
    entries = list(directory.iterdir())
    if not entries:
        raise ValueError(f"print process produced no pages in {directory}")

    numbered = []
    for path in entries:
        match = re.fullmatch(r"page-(\d{4})\.png", path.name)
        if match is None or not path.is_file():
            raise ValueError(f"unexpected print output: {path}")
        numbered.append((int(match.group(1)), path))
    numbered.sort()
    expected = list(range(1, len(numbered) + 1))
    actual = [number for number, _ in numbered]
    if actual != expected:
        raise ValueError(
            f"print page sequence must be contiguous from 1; got {actual}"
        )
    return [path for _, path in numbered]


def _select_page_paths(paths, ranges_value):
    selected = get_pages(ranges_value, len(paths))
    return [path for index, path in enumerate(paths, 1) if index in selected]


class RaikiriRefTestExecutor(RefTestExecutor):
    convert_result = reftest_result_converter

    def __init__(
        self,
        logger,
        browser,
        server_config,
        timeout_multiplier=1,
        screenshot_cache=None,
        debug_info=None,
        reftest_screenshot="unexpected",
        **kwargs,
    ):
        super().__init__(
            logger,
            browser,
            server_config,
            timeout_multiplier=timeout_multiplier,
            screenshot_cache=screenshot_cache,
            debug_info=debug_info,
            reftest_screenshot=reftest_screenshot,
            **kwargs,
        )
        self.binary = browser.binary
        self.binary_args = browser.binary_args
        self.implementation = RefTestImplementation(self)
        self.tempdir = tempfile.mkdtemp(prefix="wptrunner-raikiri-")
        self.hosts_path = os.path.join(self.tempdir, "hosts")
        with open(self.hosts_path, "w", encoding="utf-8") as hosts_file:
            hosts_file.write(make_hosts_file(self.server_config, "127.0.0.1"))
        self.proc = None
        self.command = None
        self.output = []

    def setup(self, runner, protocol=None):
        del protocol
        self.runner = runner
        self.runner.send_message("init_succeeded")
        return True

    def reset(self):
        self.implementation.reset()

    def teardown(self):
        shutil.rmtree(self.tempdir, ignore_errors=True)
        super().teardown()

    def _on_output(self, line):
        decoded = line.decode("utf-8", "replace") if isinstance(line, bytes) else line
        self.output.append(decoded.rstrip())

    def _diagnostic(self):
        message = "\n".join(item for item in self.output if item)
        return message or "Raikiri screenshot process produced no diagnostic output"

    def _run_command(self, test, command):
        self.command = command
        self.output = []
        self.proc = None
        try:
            self.proc = ProcessHandler(
                self.command,
                processOutputLine=[self._on_output],
                storeOutput=False,
            )
            self.proc.run()
            return_code = self.proc.wait(
                timeout=test.timeout * self.timeout_multiplier + self.extra_timeout
            )
        except OSError as error:
            return "ERROR", f"could not start screenshot process: {error}"
        except KeyboardInterrupt:
            if self.proc is not None:
                self.proc.kill()
            raise

        if return_code is None:
            self.proc.kill()
            self.proc.wait()
            return "EXTERNAL-TIMEOUT", self._diagnostic()
        if return_code != 0:
            return "CRASH", self._diagnostic()
        return None

    def screenshot(self, test, viewport_size, dpi, page_ranges):
        del page_ranges
        if dpi not in (None, 1, 1.0):
            return False, (
                "ERROR",
                f"Raikiri prototype does not support device-pixel-ratio {dpi}",
            )

        output_path = os.path.join(self.tempdir, f"{uuid.uuid4()}.png")
        command = [self.binary]
        command.extend(self.binary_args)
        command.extend(
            [
                "--url",
                self.test_url(test),
                "--output",
                output_path,
                "--window-size",
                viewport_size or "800x600",
                "--host-file",
                self.hosts_path,
            ]
        )
        error = self._run_command(test, command)
        if error is not None:
            return False, error
        if not os.path.isfile(output_path):
            return False, (
                "ERROR",
                f"screenshot process exited successfully without creating {output_path}",
            )

        try:
            with open(output_path, "rb") as screenshot_file:
                encoded = base64.b64encode(screenshot_file.read()).decode("ascii")
        except OSError as error:
            return False, ("ERROR", f"could not read screenshot output: {error}")
        finally:
            try:
                os.unlink(output_path)
            except OSError:
                pass
        return True, [encoded]

    def do_test(self, test):
        return self.convert_result(test, self.implementation.run_test(test))


class RaikiriPrintRefTestExecutor(RaikiriRefTestExecutor):
    is_print = True

    def screenshot(self, test, viewport_size, dpi, page_ranges):
        assert dpi is None
        width, height = _print_page_size(viewport_size or (5 * 2.54, 3 * 2.54))
        output_directory = Path(tempfile.mkdtemp(prefix="pages-", dir=self.tempdir))
        command = [self.binary]
        command.extend(self.binary_args)
        command.extend(
            [
                "print-reftest",
                "--url",
                self.test_url(test),
                "--output-directory",
                str(output_directory),
                "--page-size",
                f"{width}x{height}",
                "--host-file",
                self.hosts_path,
            ]
        )
        try:
            error = self._run_command(test, command)
            if error is not None:
                return False, error
            try:
                paths = _ordered_page_paths(output_directory)
                paths = _select_page_paths(
                    paths, (page_ranges or {}).get(test.url)
                )
                encoded = [
                    base64.b64encode(path.read_bytes()).decode("ascii")
                    for path in paths
                ]
            except (OSError, ValueError) as error:
                return False, ("ERROR", str(error))
            return True, encoded
        finally:
            shutil.rmtree(output_directory, ignore_errors=True)


def get_product():
    _allow_product_managed_host_resolution()
    return Product(
        name="raikiri",
        browser_classes={None: RaikiriBrowser},
        check_args=check_args,
        get_browser_kwargs=browser_kwargs,
        get_executor_kwargs=executor_kwargs,
        env_options={
            "server_host": "127.0.0.1",
            "bind_address": False,
            "supports_debugger": False,
        },
        get_env_extras=lambda **kwargs: [_http_only_servers],
        get_timeout_multiplier=get_timeout_multiplier,
        executor_classes={
            "reftest": RaikiriRefTestExecutor,
            "print-reftest": RaikiriPrintRefTestExecutor,
        },
    )


def _allow_product_managed_host_resolution():
    """Skip the root CLI's system-hosts check for this product only.

    The pinned root ``wpt run`` command predates a product capability for
    resolver-managed hosts and otherwise rejects every external product before
    wptrunner can create its executor. Raikiri consumes wptserve's generated
    hosts file in the screenshot process, so editing ``/etc/hosts`` is neither
    needed nor desirable.
    """
    wpt_run = sys.modules.get("tools.wpt.run")
    if wpt_run is None:
        return
    original = wpt_run.check_environ
    if getattr(original, "_raikiri_host_resolution", False):
        return

    def check_environ(product):
        if product == "raikiri":
            return None
        return original(product)

    check_environ._raikiri_host_resolution = True
    wpt_run.check_environ = check_environ


@contextlib.contextmanager
def _http_only_servers(options, config):
    """Remove secure listeners that the pinned no-SSL config leaves behind."""
    del options
    if config.ssl_config is None:
        for scheme in ("https-local", "https-public", "h2"):
            config.ports.pop(scheme, None)
    yield
