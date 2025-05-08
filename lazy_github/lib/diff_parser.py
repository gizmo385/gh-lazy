from dataclasses import dataclass
from pathlib import Path

FILE_DELETED_SENTINEL = "dev/null"


class InvalidDiffFormat(Exception):
    """An exception which is raised when we were unable to parse a supplied diff"""

    pass


@dataclass
class Hunk:
    """
    A hunk represents a roughly contiguous set of changes within a particular file
    """

    header: str
    lines: list[str]
    diff_position: int
    file_start_line: int


@dataclass
class ChangedFile:
    """
    Represents a single file that was changed by a larger diff. It is made up of hunks, as well as some metadata
    information such as the index and diff headers
    """

    path: str
    old_path: str
    deleted: bool
    hunks: list[Hunk]
    diff_header: str
    index_header: str


@dataclass
class Diff:
    """
    Represents the result of parsing an entire diff from Github. A diff contains a series of files that were changed
    by the diff
    """

    files: dict[str, ChangedFile]


@dataclass
class _ChangedFileParseResult:
    """Simple container to return from _parse_file_from_diff"""

    changed_file: ChangedFile
    updated_starting_line: int


def _parse_file_from_diff(lines: list[str], starting_line: int) -> _ChangedFileParseResult:
    # The offset tracks the current line that we're parsing, relative to the starting line in the diff for the
    # current file being parsed.
    offset = 0

    # The beginning of the diff is metadata information about the changed files.
    # 1. This is the diff header and indicates what generated the diff, as well as the files included
    diff_header = lines[starting_line + offset]
    offset += 1

    # 2. After the diff header, we have the index headers. These tell us information about how the index was changed by
    # the diff. It can span multiple lines.
    index_header_lines: list[str] = []
    while True:
        line_index = starting_line + offset
        if line_index >= len(lines):
            raise InvalidDiffFormat("Unexpected end of file while parsing index headers", diff_header)
        elif lines[line_index].startswith("--- "):
            # Once we've reached a file header, we can continue
            break
        else:
            index_header_lines.append(lines[line_index])
            offset += 1
    index_header = "\n".join(index_header_lines)

    # After the index headers, we have the from-file/to-file headers. These are generally 2 lines but can span more
    from_files: list[str] = []
    to_files: list[str] = []
    while True:
        line_index = starting_line + offset
        if line_index >= len(lines):
            raise InvalidDiffFormat("Unexpected end of file while parsing file headers", diff_header)

        current_line = lines[line_index]
        if current_line.startswith("--- "):
            from_files.append(current_line.lstrip("--- a/"))
            offset += 1
        elif current_line.startswith("+++ "):
            to_files.append(current_line.lstrip("+++ b/"))
            offset += 1
        elif current_line.startswith("@@"):
            # Once we've reached a hunk header, we can continue
            break
        else:
            raise InvalidDiffFormat(
                "Unexpected line encounterd while parsing from/to file headers",
                diff_header,
            )
    old_filename = from_files[0]
    file_deleted = to_files[0] == FILE_DELETED_SENTINEL
    filename = to_files[0] if not file_deleted else old_filename

    # After the from/to file headers, we have the hunk header, which looks like:
    # @@@ <from-file-range> <from-file-range> <to-file-range> @@@
    hunks: list[Hunk] = []
    current_hunk_header: str | None = None
    current_hunk_lines: list[str] = []
    current_hunk_diff_position = 1

    # This value is meant to snapshot the current offset before we parse the first real hunk in the file. This is
    # because all hunks in a diff have positions that are relative to the line after the first hunk header.
    first_hunk_offset_from_zero = offset
    start_line = 1

    while starting_line + offset < len(lines):
        current_line = lines[starting_line + offset]

        # If we find another hunk header, that means we've finished parsing the current hunk and can start parsing a new
        # one
        if current_line.startswith("@@"):
            if current_hunk_header and current_hunk_lines:
                new_hunk = Hunk(
                    header=current_hunk_header,
                    lines=current_hunk_lines,
                    diff_position=current_hunk_diff_position,
                    file_start_line=start_line,
                )
                hunks.append(new_hunk)
                start_line = abs(int(current_line.lstrip("@@ ").split(",")[0]))
            current_hunk_header = current_line
            current_hunk_lines = []
            current_hunk_diff_position = offset - first_hunk_offset_from_zero
            offset += 1
        elif current_line.startswith("diff"):
            # If we hit another diff header, we want to break out of the parse loop since we've finished with the
            # current file
            break
        else:
            current_hunk_lines.append(current_line)
            offset += 1

    # We need to make sure we add the current hunk, if there is one
    if current_hunk_header and current_hunk_lines:
        new_hunk = Hunk(
            header=current_hunk_header,
            lines=current_hunk_lines,
            diff_position=current_hunk_diff_position,
            file_start_line=start_line,
        )
        hunks.append(new_hunk)

    return _ChangedFileParseResult(
        updated_starting_line=starting_line + offset,
        changed_file=ChangedFile(
            path=filename,
            old_path=old_filename,
            deleted=file_deleted,
            hunks=hunks,
            diff_header=diff_header,
            index_header=index_header,
        ),
    )


def parse_diff_from_str(diff: str) -> Diff:
    """
    Given a string containing the contents of a Github API diff, parses the diff and returns a
    representation of that diff that can be traversed more programatically
    """
    lines = diff.splitlines()
    current_line = 1
    line_count = len(lines)

    changed_files: dict[str, ChangedFile] = {}
    while current_line < line_count:
        result = _parse_file_from_diff(lines, current_line)
        current_line = result.updated_starting_line
        changed_files[result.changed_file.path] = result.changed_file

    return Diff(files=changed_files)


def parse_diff_from_file(file: Path) -> Diff:
    return parse_diff_from_str(file.read_text())
