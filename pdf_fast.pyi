import os
from typing import Optional, Union

PathLike = Union[str, os.PathLike]

class PdfFastError(Exception): ...

def page_count(
    path: PathLike,
    password: Optional[bytes] = None,
    verify: bool = False,
    timeout: Optional[float] = None,
) -> int: ...
def extract_first_pages(
    src: PathLike,
    dst: PathLike,
    n: int,
    password: Optional[bytes] = None,
    max_bytes: Optional[int] = None,
    timeout: Optional[float] = None,
) -> int: ...
