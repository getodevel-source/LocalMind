#!/usr/bin/env pythonw
"""
LocalMind Studio - Native GUI Entrypoint
Runs via pythonw.exe without allocating any terminal or console window.
"""
import os
import sys
from pathlib import Path

BASE_DIR = Path(__file__).resolve().parent
os.chdir(BASE_DIR)

# Ensure virtual environment site-packages are loaded
venv_site_packages = BASE_DIR / ".venv" / "Lib" / "site-packages"
if venv_site_packages.exists():
    import site
    site.addsitedir(str(venv_site_packages))
    # Also prepend to sys.path
    if str(venv_site_packages) not in sys.path:
        sys.path.insert(0, str(venv_site_packages))

# Set virtual env environment variables
os.environ["VIRTUAL_ENV"] = str(BASE_DIR / ".venv")
venv_scripts = BASE_DIR / ".venv" / "Scripts"
if venv_scripts.exists():
    os.environ["PATH"] = str(venv_scripts) + os.pathsep + os.environ.get("PATH", "")

# Suppress standard streams in GUI mode to avoid NoneType write crashes
if sys.stdout is None:
    sys.stdout = open(os.devnull, "w", encoding="utf-8")
if sys.stderr is None:
    sys.stderr = open(os.devnull, "w", encoding="utf-8")

from main import main

if __name__ == "__main__":
    main()
