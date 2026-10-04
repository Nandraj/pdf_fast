import os
from typing import Optional, Union

PathLike = Union[str, os.PathLike]

class PdfFastError(Exception): ...

def page_count(path: PathLike, password: Optional[bytes] = None) -> int: ...
def extract_first_pages(
    src: PathLike, dst: PathLike, n: int, password: Optional[bytes] = None
) -> int: ...
