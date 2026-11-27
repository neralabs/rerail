from pathlib import Path


def is_path_in_dir(path: Path, directory: Path) -> bool:
    return path.resolve().is_relative_to(directory.resolve())


def is_file_in_dir(file: Path, directory: Path) -> bool:
    return file.is_file() and file.parent.samefile(directory)


def describe(path: Path, directory: Path) -> str:
    return f"{path} in {directory}"
