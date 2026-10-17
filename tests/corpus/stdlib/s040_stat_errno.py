# Cites: S-05-4 (rows stat, errno): names not raw numbers per row note
# Slice: stdlib-v1 (T-11, gates 02.08). Reference bytes pinned by
# /opt/homebrew/bin/python3.14 (3.14.7; provisional vs the C-01 pin 3.14.0,
# per the T-05/T-06 provenance-gap precedent).
import errno
import stat
names_s = ["S_IFDIR", "S_IFREG", "S_IFLNK", "S_ISDIR", "S_ISREG", "S_ISLNK",
           "filemode", "ST_SIZE", "ST_MODE", "ST_MTIME", "S_IRWXU",
           "S_IXGRP"]
names_e = ["errorcode", "ENOENT", "EEXIST", "EACCES", "EPERM", "EINTR",
           "EINVAL"]
print(all(hasattr(stat, n) for n in names_s),
      all(hasattr(errno, n) for n in names_e))
print(stat.filemode(stat.S_IFDIR | 0o755))
print(stat.filemode(stat.S_IFREG | 0o644))
print(stat.filemode(stat.S_IFLNK | 0o777))
print(stat.S_ISDIR(stat.S_IFDIR), stat.S_ISREG(stat.S_IFREG),
      not stat.S_ISDIR(stat.S_IFREG))
print(errno.errorcode[errno.ENOENT])
print(errno.errorcode[errno.EEXIST], errno.errorcode[errno.EACCES])
print(isinstance(errno.errorcode, dict),
      all(str(n).startswith("E") for n in errno.errorcode.values()))
mode = stat.S_IFREG | 0o644
print(stat.filemode(mode)[-9:], (mode & stat.S_IRUSR) != 0,
      (mode & stat.S_IWOTH) == 0)
print(stat.ST_SIZE is not None, stat.S_ISLNK(stat.S_IFLNK))
