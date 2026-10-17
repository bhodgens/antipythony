# Cites: S-05-4 (rows struct, heapq, bisect); S-05-3 trap (no _struct exposure)
# Slice: stdlib-v1 (T-11, gates 02.08). Reference bytes pinned by
# /opt/homebrew/bin/python3.14 (3.14.7; provisional vs the C-01 pin 3.14.0,
# per the T-05/T-06 provenance-gap precedent).
import bisect
import heapq
import struct
names_s = ["pack", "unpack", "pack_into", "unpack_from", "calcsize",
           "iter_unpack", "Struct", "error"]
names_h = ["heappush", "heappop", "heapify", "heapreplace", "heappushpop",
           "merge", "nlargest", "nsmallest", "heapify_max", "heappop_max",
           "heappush_max", "heappushpop_max", "heapreplace_max"]
names_b = ["bisect", "bisect_left", "bisect_right", "insort", "insort_left",
           "insort_right"]
print(all(hasattr(struct, n) for n in names_s),
      all(hasattr(heapq, n) for n in names_h),
      all(hasattr(bisect, n) for n in names_b))
print(struct.calcsize(">2h"), struct.pack(">2h", 1, 2).hex())
print(struct.pack("!Bhl", 1, -2, 3).hex())
print(struct.Struct("<i").size)
print(struct.unpack("2B", b"\x01\x02"),
      struct.unpack_from(">h", b"\x00\x05", 0))
ba = bytearray(4)
struct.pack_into(">i", ba, 0, 258)
print(ba.hex(), struct.unpack_from(">i", ba)[0])
print(list(struct.iter_unpack("<2b", b"\x01\x02\x03\x04")))
try:
    struct.unpack("i", b"xx")
except struct.error as e:
    print("struct.error:", e)
try:
    struct.pack("Q", -1)
except struct.error as e:
    print("struct.error2:", e)
h = []
for v in [5, 3, 8, 1]:
    heapq.heappush(h, v)
print(h[0], [heapq.heappop(h) for _ in range(4)])
hh = [9, 4, 7]
heapq.heapify(hh)
print(hh, heapq.heappushpop(hh, 2), heapq.heapreplace(hh, 0))
print(heapq.nlargest(3, [1, 5, 3, 9, 2]), heapq.nsmallest(2, [1, 5, 3, 9, 2]))
print(list(heapq.merge([1, 4], [2, 3], [0])))
hmax = [1, 5, 3]
heapq.heapify_max(hmax)
print(hmax[0], heapq.heappop_max(hmax))
a = [1, 3, 5, 7]
print(bisect.bisect_left(a, 2), bisect.bisect(a, 5), bisect.bisect_right(a, 5))
bisect.insort(a, 4)
print(a)
bisect.insort_left(a, 4)
print(a)
print(bisect.bisect(a, 4, lo=1), issubclass(struct.error, Exception))
