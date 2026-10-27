"""Hatch build hook: compile the Rust binary and bundle it in the wheel."""

import shutil
import subprocess
from pathlib import Path

from hatchling.builders.hooks.plugin.interface import BuildHookInterface


class CustomBuildHook(BuildHookInterface):
    """Build the linux-gui-harness Rust binary and copy it into the package."""

    def initialize(self, version: str, build_data: dict) -> None:
        project_root = Path(self.root).resolve()
        binary_name = "linux-gui-harness"

        subprocess.run(
            ["cargo", "build", "--release"],
            cwd=str(project_root),
            check=True,
        )

        src = project_root / "target" / "release" / binary_name
        if not src.exists():
            raise RuntimeError(f"Built binary not found: {src}")

        pkg_dir = project_root / "python" / "src" / "linux_gui_harness"
        bin_dir = pkg_dir / "bin"
        bin_dir.mkdir(parents=True, exist_ok=True)
        dst = bin_dir / binary_name

        shutil.copy2(src, dst)
        dst.chmod(0o755)

        skill_src = project_root / "skills" / "linux-gui-harness"
        skill_dst = pkg_dir / "skill"
        if skill_dst.exists():
            shutil.rmtree(skill_dst)
        shutil.copytree(skill_src, skill_dst)

        lifecycle_src = project_root / "lifecycle"
        lifecycle_dst = pkg_dir / "lifecycle"
        if lifecycle_dst.exists():
            shutil.rmtree(lifecycle_dst)
        if lifecycle_src.exists():
            shutil.copytree(lifecycle_src, lifecycle_dst)

        build_data["tag"] = "py3-none-linux_x86_64"
        build_data["pure_python"] = False
