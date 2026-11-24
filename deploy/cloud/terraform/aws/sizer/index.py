"""Sizes an fctl runner pool's Auto Scaling group to what the pool needs now.

Every minute (an EventBridge schedule) it reads the pool's demand with its
enrollment token (GET /api/v1/runner/enroll/demand), puts WaitingSessions,
BusySlots and WantedRunners into CloudWatch, and sets the group's desired
capacity to `wanted`, within the group's limits. Where the machines retire
themselves once idle (RAISE_ONLY), it only ever raises it, so that the group
never takes a busy machine.
"""

import json
import os
import urllib.request

import boto3

autoscaling = boto3.client("autoscaling")
cloudwatch = boto3.client("cloudwatch")
secrets = boto3.client("secretsmanager")


def demand(server, token):
    request = urllib.request.Request(
        server.rstrip("/") + "/api/v1/runner/enroll/demand",
        headers={"Authorization": f"Bearer {token}", "User-Agent": "fctl-pool-sizer"},
    )
    with urllib.request.urlopen(request, timeout=20) as answer:
        return json.load(answer)


def pool_dimension(pool):
    """The pool's name, or its id where CloudWatch wouldn't take the name."""
    name = pool.get("name", "")
    if name.strip() and len(name) <= 1024 and all(32 <= ord(c) < 127 for c in name):
        return name
    return pool["id"]


def handler(event, context):
    token = secrets.get_secret_value(SecretId=os.environ["SECRET"])["SecretString"]
    d = demand(os.environ["FCTL_SERVER"], token)
    wanted = int(d["wanted"])
    dimensions = [{"Name": "Pool", "Value": pool_dimension(d["pool"])}]
    cloudwatch.put_metric_data(
        Namespace=os.environ.get("NAMESPACE", "fctl"),
        MetricData=[
            {"MetricName": name, "Dimensions": dimensions, "Value": value, "Unit": "Count"}
            for name, value in (
                ("WaitingSessions", d["waiting"]["sessions"]),
                ("BusySlots", d["slots"]["busy"]),
                ("WantedRunners", wanted),
            )
        ],
    )
    group_name = os.environ["GROUP"]
    group = autoscaling.describe_auto_scaling_groups(AutoScalingGroupNames=[group_name])["AutoScalingGroups"][0]
    target = min(max(wanted, group["MinSize"]), group["MaxSize"])
    current = group["DesiredCapacity"]
    raise_only = os.environ.get("RAISE_ONLY") == "1"
    if target > current or (target < current and not raise_only):
        autoscaling.set_desired_capacity(
            AutoScalingGroupName=group_name, DesiredCapacity=target, HonorCooldown=False
        )
        current = target
    return {"wanted": wanted, "desired": current}
