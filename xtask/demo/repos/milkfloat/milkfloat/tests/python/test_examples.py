"""Every script in examples/ runs: the docs' examples can't go stale."""

import pathlib
import runpy

import pytest

EXAMPLES = sorted((pathlib.Path(__file__).parents[2] / "examples").glob("*.py"))


@pytest.mark.parametrize("path", EXAMPLES, ids=lambda p: p.name)
def test_example_runs(path, capsys):
    runpy.run_path(str(path), run_name="__main__")
    assert capsys.readouterr().out.strip()
