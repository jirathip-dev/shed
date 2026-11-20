#!/usr/bin/env python3
"""What a server sees when a peer stops writing, and when it closes its whole socket (adr.md, "Checked by running").

A Unix stream socket, watched edge-triggered for reading and writing as tokio watches it: kqueue on macOS,
epoll on Linux. Prints the events after each step.
"""

import os
import select
import socket
import tempfile

folder = tempfile.mkdtemp()
path = os.path.join(folder, "s")
server = socket.socket(socket.AF_UNIX)
server.bind(path)
server.listen(1)
peer = socket.socket(socket.AF_UNIX)
peer.connect(path)
ours, _ = server.accept()
ours.setblocking(False)

if hasattr(select, "kqueue"):
    queue = select.kqueue()
    add = select.KQ_EV_ADD | select.KQ_EV_CLEAR
    queue.control([select.kevent(ours, f, add) for f in (select.KQ_FILTER_READ, select.KQ_FILTER_WRITE)], 0)
    names = {select.KQ_FILTER_READ: "read", select.KQ_FILTER_WRITE: "write"}

    def events():
        return [names[e.filter] + (" closed" if e.flags & select.KQ_EV_EOF else " ready") for e in queue.control(None, 8, 0.3)]
else:
    queue = select.epoll()
    queue.register(ours, select.EPOLLIN | select.EPOLLOUT | select.EPOLLRDHUP | select.EPOLLET)
    flags = [(select.EPOLLIN, "read ready"), (select.EPOLLOUT, "write ready"), (select.EPOLLRDHUP, "read closed"), (select.EPOLLHUP, "write closed")]

    def events():
        return [name for _, mask in queue.poll(0.3) for flag, name in flags if mask & flag]


print("connected:            ", events())
peer.sendall(b"{}\n")
print("the peer wrote a line:", events())
peer.shutdown(socket.SHUT_WR)
print("it stopped writing:   ", events())
print("still only that:      ", events())
peer.close()
print("it closed its socket: ", events())
os.unlink(path)
os.rmdir(folder)
