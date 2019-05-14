FROM ubuntu:bionic
RUN apt-get update
RUN apt-get install -y  openssh-server haveged udev
RUN apt-get clean

COPY rsinit /sbin/
EXPOSE 22 

RUN mkdir /run/sshd

CMD ["/sbin/rsinit"]
