#!/usr/bin/python3
#   Copyright 2020 The KNIX Authors
#
#   Licensed under the Apache License, Version 2.0 (the "License");
#   you may not use this file except in compliance with the License.
#   You may obtain a copy of the License at
#
#       http://www.apache.org/licenses/LICENSE-2.0
#
#   Unless required by applicable law or agreed to in writing, software
#   distributed under the License is distributed on an "AS IS" BASIS,
#   WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
#   See the License for the specific language governing permissions and
#   limitations under the License.


""" Add a host to the 'available_hosts' key of the management service
"""
import os
import json
import sys
import riak
import socket
import subprocess

### global variables set at runtime
DLCLIENT=None
BUCKET=None
BUCKETNAME=''

def get_storage_userid(email):
    storage_userid = email.replace("@", "AT")
    storage_userid = storage_userid.replace(".", "_")
    storage_userid = storage_userid.replace("-", "_")
    return storage_userid

def set_bucket_name(sid,wid):
    global BUCKETNAME
    #keyspace = "storage_" + get_storage_userid(email)
    #tablename = "defaultTable"
    keyspace = "sbox_"+sid
    tablename = "wf_"+wid
    BUCKETNAME = keyspace + ";" + tablename
    return BUCKETNAME

def dl_get(key):
    global DLCLIENT
    global BUCKET
    global BUCKETNAME
    if DLCLIENT is None:
        host,port = os.getenv("RIAK_CONNECT",socket.gethostname()+":8087").split(":")
        print("Connecting to Riak server at "+host+":"+port)
        DLCLIENT = riak.RiakClient(protocol='pbc',host=host,port=port)
    if BUCKET is None:
        # If the bucket does not exist, Riak will create it for you.
        BUCKET = DLCLIENT.bucket_type('default').bucket(BUCKETNAME)
    obj = BUCKET.get(key)
    return obj


def add_host(hostname,hostip=None):
    if hostip is None:
        hostip = socket.gethostbyname(hostname)
    has_gpu = False
    try:
        has_gpu = ("NVIDIA" in subprocess.check_output('nvcc --version.split(' ')).decode()'))
    except Exception:
        print("No suitable GPU available on this host!")
        pass

    print("Adding host: " + str(hostname) + ", has gpu: "+ str(has_gpu))

    v = dl_get("available_hosts")
    if v.encoded_data is not None and len(v.encoded_data) > 0:
        hosts = json.loads((v.encoded_data).decode())
        print("existing hosts: " + str(hosts))
        if isinstance(hosts,list):
            hosts = {host: socket.gethostbyname(host) for host in hosts}
    else:
        hosts = {}
    if hostname != None and hostname not in hosts:
        hosts[hostname] = hostip
        hosts[hostname]["has_gpu"] = True
        v.encoded_data = json.dumps(hosts).encode()
        v.store()
    return hosts


def remove_host(hostname):
    print("Removing host: " + str(hostname))
    v = dl_get("available_hosts")
    if v.encoded_data is not None and len(v.encoded_data) > 0:
        hosts = json.loads((v.encoded_data).decode())
        if isinstance(hosts,list):
            hosts = {host: socket.gethostbyname(host) for host in hosts}
    else:
        hosts = {}
    if hostname != None and hostname in hosts:
        del hosts[hostname]
        v.encoded_data = json.dumps(hosts).encode()
        v.store()
    return hosts


if __name__ == "__main__":
    defaulthost = os.getenv("MFN_HOSTNAME", socket.gethostname().split('.',1)[0])
    email = "admin@management"
    sandboxid = "Management"
    workflowid = "Management"
    hosts = []
    set_bucket_name(sandboxid,workflowid)
    try:
        host=defaulthost
        if len(sys.argv) > 2:
            host = sys.argv[2]
        if sys.argv[1] == "add":
            hosts = add_host(host)
        elif sys.argv[1] == "remove":
            hosts = remove_host(host)
        else:
            raise Exception()
    except Exception as e:
        print(e)
        print("usage: python "+sys.argv[0]+" [add|remove] (<hostname>)")
        print("  optional <hostname> defaults to %s" % defaulthost)
        v = dl_get("available_hosts")
        if v.encoded_data is not None and len(v.encoded_data) > 0:
            hosts = json.loads((v.encoded_data).decode())
    print("Current available_hosts=" + str(hosts))

