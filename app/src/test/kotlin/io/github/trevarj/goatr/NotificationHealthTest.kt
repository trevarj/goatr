// SPDX-License-Identifier: GPL-3.0-or-later
package io.github.trevarj.goatr

import android.app.NotificationManager
import org.junit.Assert.*
import org.junit.Test

/** Production admission/status/attempt decisions only, never evidence of OS delivery or FGS execution. */
class NotificationHealthTest {
    private fun health(api:Int=34,app:Boolean=true,granted:Boolean=true,attention:Int?=NotificationManager.IMPORTANCE_DEFAULT,delivery:Int?=NotificationManager.IMPORTANCE_LOW)=
        NotificationHealth.measured(api,app,granted,attention,delivery)

    @Test fun independentChannelBlocksChooseActualAttemptStatusAndRepairConsumers() {
        data class Case(val health:NotificationHealth,val attentionAttempts:Int,val keeperAttempts:Int,val state:NotificationDeliveryState,val repairs:List<NotificationRepair>)
        val cases=listOf(
            Case(health(),1,1,NotificationDeliveryState.Connected,emptyList()),
            Case(health(attention=0),0,1,NotificationDeliveryState.AttentionBlocked,listOf(NotificationRepair.Attention)),
            Case(health(delivery=0),1,0,NotificationDeliveryState.DeliveryBlocked,listOf(NotificationRepair.Delivery)),
            Case(health(attention=0,delivery=0),0,0,NotificationDeliveryState.BothChannelsBlocked,listOf(NotificationRepair.Attention,NotificationRepair.Delivery)),
            Case(health(app=false),0,0,NotificationDeliveryState.GlobalBlocked,listOf(NotificationRepair.App)),
            Case(health(app=false,attention=0,delivery=0),0,0,NotificationDeliveryState.GlobalBlocked,NotificationRepair.entries),
        )
        for(case in cases) {
            var attentionAttempts=0;var keeperAttempts=0
            val attention=notificationAttempt({case.health.allowsAttention(true)}){attentionAttempts++}
            val keeper=notificationAttempt({case.health.keeperAvailable}){keeperAttempts++}
            assertEquals(case.attentionAttempts,attentionAttempts);assertEquals(case.keeperAttempts,keeperAttempts)
            assertEquals(if(case.attentionAttempts==1)PresentationOutcome.AttemptedHealthy else PresentationOutcome.BlockedBeforeAttempt,attention)
            assertEquals(if(case.keeperAttempts==1)PresentationOutcome.AttemptedHealthy else PresentationOutcome.BlockedBeforeAttempt,keeper)
            assertEquals(case.state,case.health.deliveryState(true,true,true,true,false))
            assertEquals(case.repairs,case.health.repairs())
        }
    }

    @Test fun apiThirtyNeverRequestsRuntimePermissionButApiThirtyFourMissingGrantBlocksBothEffects() {
        val old=health(api=30,granted=false)
        val current=health(api=34,granted=false)
        var posts=0;var foregroundStarts=0
        assertFalse(old.runtimePermissionRequired)
        assertEquals(PresentationOutcome.AttemptedHealthy,notificationAttempt({old.allowsAttention(true)}){posts++})
        assertEquals(PresentationOutcome.AttemptedHealthy,notificationAttempt({old.keeperAvailable}){foregroundStarts++})
        assertTrue(current.runtimePermissionRequired)
        assertEquals(PresentationOutcome.BlockedBeforeAttempt,notificationAttempt({current.allowsAttention(true)}){posts++})
        assertEquals(PresentationOutcome.BlockedBeforeAttempt,notificationAttempt({current.keeperAvailable}){foregroundStarts++})
        assertEquals(1,posts);assertEquals(1,foregroundStarts)
        assertEquals(NotificationDeliveryState.GlobalBlocked,current.deliveryState(true,true,true,true,false))
        assertEquals(listOf(NotificationRepair.App),current.repairs())
        val channelOnly=health(api=30,granted=false,delivery=0)
        assertFalse(channelOnly.runtimePermissionRequired)
        assertEquals(listOf(NotificationRepair.Delivery),channelOnly.repairs())
        assertEquals(PresentationOutcome.BlockedBeforeAttempt,notificationAttempt({channelOnly.keeperAvailable}){foregroundStarts++})
        assertEquals(1,foregroundStarts)
    }

    @Test fun policyToPreAttemptBlockMakesNoEffectButDuringAttemptChangeOrThrowIsUnknown() {
        var current=health();var effects=0;var checks=0
        // Policy was healthy; immediate post admission is independently measured later.
        assertEquals(NotificationDeliveryState.Connected,current.deliveryState(true,true,true,true,false))
        current=health(attention=0)
        assertEquals(PresentationOutcome.BlockedBeforeAttempt,notificationAttempt({checks++;current.allowsAttention(true)}){effects++})
        assertEquals(0,effects);assertEquals(1,checks)
        current=health();checks=0
        assertEquals(PresentationOutcome.AttemptedUnknown,notificationAttempt({checks++;current.allowsAttention(true)}){effects++;current=health(attention=0)})
        assertEquals(1,effects);assertEquals(2,checks)
        current=health();checks=0
        assertEquals(PresentationOutcome.AttemptedUnknown,notificationAttempt({checks++;current.allowsAttention(true)}){effects++;throw IllegalStateException("Owned effect boundary")})
        assertEquals(2,effects);assertEquals(1,checks)
        checks=0
        assertEquals(PresentationOutcome.AttemptedUnknown,notificationAttempt({checks++;if(checks==2)throw IllegalStateException("Owned post-health boundary");true}){effects++})
        assertEquals(3,effects);assertEquals(2,checks)
    }

    @Test fun keeperHealthLossCannotPreserveConnectedStatusOrReclassifySocketFreeRegistration() {
        val blocked=health(delivery=0)
        var starts=0
        assertEquals(PresentationOutcome.BlockedBeforeAttempt,notificationAttempt({blocked.keeperAvailable}){starts++})
        assertEquals(0,starts)
        assertEquals(NotificationDeliveryState.DeliveryBlocked,blocked.deliveryState(true,true,true,true,false))
        assertEquals(NotificationDeliveryState.SocketFree,blocked.deliveryState(true,false,true,false,true))
        assertEquals(listOf(NotificationRepair.Delivery),blocked.repairs())
        assertEquals(NotificationDeliveryState.AttentionBlocked,health(attention=0,delivery=0).deliveryState(true,false,true,false,false))
        var current=health()
        assertEquals(PresentationOutcome.AttemptedUnknown,notificationAttempt({current.keeperAvailable}){starts++;current=blocked})
        assertEquals(1,starts)
        assertEquals(NotificationDeliveryState.DeliveryBlocked,current.deliveryState(true,true,true,true,false))
        assertEquals(NotificationDeliveryState.Stopped,health().deliveryState(true,true,true,false,false))
        assertEquals(NotificationDeliveryState.Reconnecting,health().deliveryState(true,true,true,true,true))
    }

    @Test fun missingChannelsAndDisabledHostNeverAdmitAttentionOrFalsifyOffStatus() {
        val absent=health(attention=null,delivery=null)
        var effects=0
        assertEquals(PresentationOutcome.BlockedBeforeAttempt,notificationAttempt({absent.allowsAttention(true)}){effects++})
        assertEquals(PresentationOutcome.BlockedBeforeAttempt,notificationAttempt({absent.keeperAvailable}){effects++})
        assertEquals(listOf(NotificationRepair.Attention,NotificationRepair.Delivery),absent.repairs())
        assertEquals(PresentationOutcome.BlockedBeforeAttempt,notificationAttempt({health().allowsAttention(false)}){effects++})
        assertEquals(NotificationDeliveryState.Off,absent.deliveryState(false,true,true,true,false))
        assertEquals(0,effects)
    }
}
