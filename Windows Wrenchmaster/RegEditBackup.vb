Imports System.Management
Imports System.Windows.Forms

Public Class RegEditBackup
    Private Sub FlatButton2_Click(sender As Object, e As EventArgs) Handles FlatButton2.Click
        Me.Close()
    End Sub

    Private Sub FlatButton1_Click(sender As Object, e As EventArgs) Handles FlatButton1.Click
        FlatButton1.Enabled = False
        Try
            Using restoreClass As New ManagementClass(New ManagementPath("\\.\root\default:SystemRestore"))
                Dim input As ManagementBaseObject = restoreClass.GetMethodParameters("CreateRestorePoint")
                input("Description") = "Wrenchmaster manual restore point"
                input("RestorePointType") = 12
                input("EventType") = 100
                Using result As ManagementBaseObject = restoreClass.InvokeMethod("CreateRestorePoint", input, Nothing)
                    Dim returnCode As UInteger = Convert.ToUInt32(result("ReturnValue"))
                    If returnCode = 0 Then
                        MessageBox.Show("The restore point was created successfully.", "System Restore", MessageBoxButtons.OK, MessageBoxIcon.Information)
                    Else
                        MessageBox.Show("Windows could not create the restore point (code " & returnCode.ToString() & "). Check that System Protection is enabled and try again.", "System Restore", MessageBoxButtons.OK, MessageBoxIcon.Warning)
                    End If
                End Using
            End Using
        Catch ex As Exception
            MessageBox.Show("Windows could not create a restore point. Check that System Protection is enabled. Details: " & ex.Message, "System Restore", MessageBoxButtons.OK, MessageBoxIcon.Error)
        Finally
            FlatButton1.Enabled = True
        End Try
        Me.Close()
    End Sub
End Class
