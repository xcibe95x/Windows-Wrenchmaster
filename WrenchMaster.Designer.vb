<Global.Microsoft.VisualBasic.CompilerServices.DesignerGenerated()> _
Partial Class WrenchMaster
    Inherits System.Windows.Forms.Form

    'Form esegue l'override del metodo Dispose per pulire l'elenco dei componenti.
    <System.Diagnostics.DebuggerNonUserCode()> _
    Protected Overrides Sub Dispose(ByVal disposing As Boolean)
        Try
            If disposing AndAlso components IsNot Nothing Then
                components.Dispose()
            End If
        Finally
            MyBase.Dispose(disposing)
        End Try
    End Sub

    'Richiesto da Progettazione Windows Form
    Private components As System.ComponentModel.IContainer

    'NOTA: la procedura che segue è richiesta da Progettazione Windows Form
    'Può essere modificata in Progettazione Windows Form.  
    'Non modificarla nell'editor del codice.
    <System.Diagnostics.DebuggerStepThrough()> _
    Private Sub InitializeComponent()
        Me.components = New System.ComponentModel.Container()
        Dim resources As System.ComponentModel.ComponentResourceManager = New System.ComponentModel.ComponentResourceManager(GetType(WrenchMaster))
        Me.LogCleaner = New System.Windows.Forms.Timer(Me.components)
        Me.FormSkin1 = New Windows_Wrenchmaster.FormSkin()
        Me.FlatStickyButton2 = New Windows_Wrenchmaster.FlatStickyButton()
        Me.FlatStickyButton1 = New Windows_Wrenchmaster.FlatStickyButton()
        Me.FlatTabControl1 = New Windows_Wrenchmaster.FlatTabControl()
        Me.TabPage1 = New System.Windows.Forms.TabPage()
        Me.FlatButton7 = New Windows_Wrenchmaster.FlatButton()
        Me.FlatButton6 = New Windows_Wrenchmaster.FlatButton()
        Me.FlatButton5 = New Windows_Wrenchmaster.FlatButton()
        Me.FlatCheckBox2 = New Windows_Wrenchmaster.FlatCheckBox()
        Me.FlatCheckBox1 = New Windows_Wrenchmaster.FlatCheckBox()
        Me.FlatGroupBox1 = New Windows_Wrenchmaster.FlatGroupBox()
        Me.Label2 = New System.Windows.Forms.Label()
        Me.Label1 = New System.Windows.Forms.Label()
        Me.CleanerProgress = New Windows_Wrenchmaster.CircleProgressBarBigText()
        Me.FlatAlertBox11 = New Windows_Wrenchmaster.FlatAlertBox()
        Me.FlatAlertBox8 = New Windows_Wrenchmaster.FlatAlertBox()
        Me.FlatAlertBox9 = New Windows_Wrenchmaster.FlatAlertBox()
        Me.FlatAlertBox10 = New Windows_Wrenchmaster.FlatAlertBox()
        Me.FlatAlertBox5 = New Windows_Wrenchmaster.FlatAlertBox()
        Me.FlatAlertBox6 = New Windows_Wrenchmaster.FlatAlertBox()
        Me.FlatAlertBox7 = New Windows_Wrenchmaster.FlatAlertBox()
        Me.FlatAlertBox3 = New Windows_Wrenchmaster.FlatAlertBox()
        Me.FlatAlertBox4 = New Windows_Wrenchmaster.FlatAlertBox()
        Me.FlatAlertBox2 = New Windows_Wrenchmaster.FlatAlertBox()
        Me.FlatAlertBox1 = New Windows_Wrenchmaster.FlatAlertBox()
        Me.TabPage2 = New System.Windows.Forms.TabPage()
        Me.TabPage4 = New System.Windows.Forms.TabPage()
        Me.TabPage3 = New System.Windows.Forms.TabPage()
        Me.TabPage5 = New System.Windows.Forms.TabPage()
        Me.TabPage6 = New System.Windows.Forms.TabPage()
        Me.FlatTextBox1 = New Windows_Wrenchmaster.FlatTextBox()
        Me.FlatButton4 = New Windows_Wrenchmaster.FlatButton()
        Me.FlatButton3 = New Windows_Wrenchmaster.FlatButton()
        Me.TabPage7 = New System.Windows.Forms.TabPage()
        Me.VideoCard = New Windows_Wrenchmaster.FlatLabel()
        Me.InfoLabel = New Windows_Wrenchmaster.FlatLabel()
        Me.WinBitsLabel = New Windows_Wrenchmaster.FlatLabel()
        Me.AppBitsLabel = New Windows_Wrenchmaster.FlatLabel()
        Me.WindowsLabel = New Windows_Wrenchmaster.FlatLabel()
        Me.FlatStatusBar1 = New Windows_Wrenchmaster.FlatStatusBar()
        Me.FormSkin1.SuspendLayout()
        Me.FlatTabControl1.SuspendLayout()
        Me.TabPage1.SuspendLayout()
        Me.FlatGroupBox1.SuspendLayout()
        Me.TabPage6.SuspendLayout()
        Me.TabPage7.SuspendLayout()
        Me.SuspendLayout()
        '
        'LogCleaner
        '
        '
        'FormSkin1
        '
        Me.FormSkin1.BackColor = System.Drawing.Color.White
        Me.FormSkin1.BaseColor = System.Drawing.Color.FromArgb(CType(CType(45, Byte), Integer), CType(CType(47, Byte), Integer), CType(CType(49, Byte), Integer))
        Me.FormSkin1.BorderColor = System.Drawing.Color.FromArgb(CType(CType(33, Byte), Integer), CType(CType(38, Byte), Integer), CType(CType(39, Byte), Integer))
        Me.FormSkin1.Controls.Add(Me.FlatStickyButton2)
        Me.FormSkin1.Controls.Add(Me.FlatStickyButton1)
        Me.FormSkin1.Controls.Add(Me.FlatTabControl1)
        Me.FormSkin1.Controls.Add(Me.VideoCard)
        Me.FormSkin1.Controls.Add(Me.InfoLabel)
        Me.FormSkin1.Controls.Add(Me.WinBitsLabel)
        Me.FormSkin1.Controls.Add(Me.AppBitsLabel)
        Me.FormSkin1.Controls.Add(Me.WindowsLabel)
        Me.FormSkin1.Controls.Add(Me.FlatStatusBar1)
        Me.FormSkin1.Dock = System.Windows.Forms.DockStyle.Fill
        Me.FormSkin1.FlatColor = System.Drawing.Color.FromArgb(CType(CType(35, Byte), Integer), CType(CType(168, Byte), Integer), CType(CType(109, Byte), Integer))
        Me.FormSkin1.Font = New System.Drawing.Font("Segoe UI", 12.0!)
        Me.FormSkin1.HeaderColor = System.Drawing.Color.FromArgb(CType(CType(33, Byte), Integer), CType(CType(38, Byte), Integer), CType(CType(39, Byte), Integer))
        Me.FormSkin1.HeaderMaximize = False
        Me.FormSkin1.Location = New System.Drawing.Point(0, 0)
        Me.FormSkin1.Name = "FormSkin1"
        Me.FormSkin1.Size = New System.Drawing.Size(943, 564)
        Me.FormSkin1.TabIndex = 2
        Me.FormSkin1.Text = "Wrenchmaster"
        '
        'FlatStickyButton2
        '
        Me.FlatStickyButton2.BackColor = System.Drawing.Color.Transparent
        Me.FlatStickyButton2.BaseColor = System.Drawing.Color.FromArgb(CType(CType(33, Byte), Integer), CType(CType(38, Byte), Integer), CType(CType(39, Byte), Integer))
        Me.FlatStickyButton2.Cursor = System.Windows.Forms.Cursors.Hand
        Me.FlatStickyButton2.Font = New System.Drawing.Font("Segoe UI", 12.0!, System.Drawing.FontStyle.Underline)
        Me.FlatStickyButton2.Location = New System.Drawing.Point(895, 0)
        Me.FlatStickyButton2.Name = "FlatStickyButton2"
        Me.FlatStickyButton2.Rounded = False
        Me.FlatStickyButton2.Size = New System.Drawing.Size(24, 22)
        Me.FlatStickyButton2.TabIndex = 17
        Me.FlatStickyButton2.Text = "_"
        Me.FlatStickyButton2.TextColor = System.Drawing.Color.FromArgb(CType(CType(243, Byte), Integer), CType(CType(243, Byte), Integer), CType(CType(243, Byte), Integer))
        '
        'FlatStickyButton1
        '
        Me.FlatStickyButton1.BackColor = System.Drawing.Color.Transparent
        Me.FlatStickyButton1.BaseColor = System.Drawing.Color.FromArgb(CType(CType(33, Byte), Integer), CType(CType(38, Byte), Integer), CType(CType(39, Byte), Integer))
        Me.FlatStickyButton1.Cursor = System.Windows.Forms.Cursors.Hand
        Me.FlatStickyButton1.Font = New System.Drawing.Font("Segoe UI", 20.0!, System.Drawing.FontStyle.Regular, System.Drawing.GraphicsUnit.Pixel)
        Me.FlatStickyButton1.Location = New System.Drawing.Point(919, 0)
        Me.FlatStickyButton1.Name = "FlatStickyButton1"
        Me.FlatStickyButton1.Rounded = False
        Me.FlatStickyButton1.Size = New System.Drawing.Size(24, 22)
        Me.FlatStickyButton1.TabIndex = 0
        Me.FlatStickyButton1.Text = "×"
        Me.FlatStickyButton1.TextColor = System.Drawing.Color.FromArgb(CType(CType(243, Byte), Integer), CType(CType(243, Byte), Integer), CType(CType(243, Byte), Integer))
        '
        'FlatTabControl1
        '
        Me.FlatTabControl1.ActiveColor = System.Drawing.Color.MediumSlateBlue
        Me.FlatTabControl1.Appearance = System.Windows.Forms.TabAppearance.Buttons
        Me.FlatTabControl1.BaseColor = System.Drawing.Color.FromArgb(CType(CType(33, Byte), Integer), CType(CType(38, Byte), Integer), CType(CType(39, Byte), Integer))
        Me.FlatTabControl1.Controls.Add(Me.TabPage1)
        Me.FlatTabControl1.Controls.Add(Me.TabPage2)
        Me.FlatTabControl1.Controls.Add(Me.TabPage4)
        Me.FlatTabControl1.Controls.Add(Me.TabPage3)
        Me.FlatTabControl1.Controls.Add(Me.TabPage5)
        Me.FlatTabControl1.Controls.Add(Me.TabPage6)
        Me.FlatTabControl1.Controls.Add(Me.TabPage7)
        Me.FlatTabControl1.Dock = System.Windows.Forms.DockStyle.Bottom
        Me.FlatTabControl1.DrawMode = System.Windows.Forms.TabDrawMode.OwnerDrawFixed
        Me.FlatTabControl1.Font = New System.Drawing.Font("Segoe UI", 10.0!)
        Me.FlatTabControl1.ImeMode = System.Windows.Forms.ImeMode.NoControl
        Me.FlatTabControl1.ItemSize = New System.Drawing.Size(120, 40)
        Me.FlatTabControl1.Location = New System.Drawing.Point(0, 48)
        Me.FlatTabControl1.Name = "FlatTabControl1"
        Me.FlatTabControl1.SelectedIndex = 0
        Me.FlatTabControl1.Size = New System.Drawing.Size(943, 493)
        Me.FlatTabControl1.SizeMode = System.Windows.Forms.TabSizeMode.Fixed
        Me.FlatTabControl1.TabIndex = 16
        Me.FlatTabControl1.TabStop = False
        '
        'TabPage1
        '
        Me.TabPage1.BackColor = System.Drawing.Color.FromArgb(CType(CType(45, Byte), Integer), CType(CType(47, Byte), Integer), CType(CType(49, Byte), Integer))
        Me.TabPage1.Controls.Add(Me.FlatButton7)
        Me.TabPage1.Controls.Add(Me.FlatButton6)
        Me.TabPage1.Controls.Add(Me.FlatButton5)
        Me.TabPage1.Controls.Add(Me.FlatCheckBox2)
        Me.TabPage1.Controls.Add(Me.FlatCheckBox1)
        Me.TabPage1.Controls.Add(Me.FlatGroupBox1)
        Me.TabPage1.Controls.Add(Me.FlatAlertBox11)
        Me.TabPage1.Controls.Add(Me.FlatAlertBox8)
        Me.TabPage1.Controls.Add(Me.FlatAlertBox9)
        Me.TabPage1.Controls.Add(Me.FlatAlertBox10)
        Me.TabPage1.Controls.Add(Me.FlatAlertBox5)
        Me.TabPage1.Controls.Add(Me.FlatAlertBox6)
        Me.TabPage1.Controls.Add(Me.FlatAlertBox7)
        Me.TabPage1.Controls.Add(Me.FlatAlertBox3)
        Me.TabPage1.Controls.Add(Me.FlatAlertBox4)
        Me.TabPage1.Controls.Add(Me.FlatAlertBox2)
        Me.TabPage1.Controls.Add(Me.FlatAlertBox1)
        Me.TabPage1.Location = New System.Drawing.Point(4, 44)
        Me.TabPage1.Name = "TabPage1"
        Me.TabPage1.Padding = New System.Windows.Forms.Padding(3)
        Me.TabPage1.Size = New System.Drawing.Size(935, 445)
        Me.TabPage1.TabIndex = 0
        Me.TabPage1.Text = "Cleaner"
        '
        'FlatButton7
        '
        Me.FlatButton7.BackColor = System.Drawing.Color.Transparent
        Me.FlatButton7.BaseColor = System.Drawing.Color.MediumSlateBlue
        Me.FlatButton7.Cursor = System.Windows.Forms.Cursors.Hand
        Me.FlatButton7.Font = New System.Drawing.Font("Segoe UI", 12.0!)
        Me.FlatButton7.Location = New System.Drawing.Point(447, 389)
        Me.FlatButton7.Name = "FlatButton7"
        Me.FlatButton7.Rounded = False
        Me.FlatButton7.Size = New System.Drawing.Size(155, 47)
        Me.FlatButton7.TabIndex = 17
        Me.FlatButton7.Text = "Settings"
        Me.FlatButton7.TextColor = System.Drawing.Color.FromArgb(CType(CType(243, Byte), Integer), CType(CType(243, Byte), Integer), CType(CType(243, Byte), Integer))
        '
        'FlatButton6
        '
        Me.FlatButton6.BackColor = System.Drawing.Color.Transparent
        Me.FlatButton6.BaseColor = System.Drawing.Color.Gray
        Me.FlatButton6.Cursor = System.Windows.Forms.Cursors.Hand
        Me.FlatButton6.Font = New System.Drawing.Font("Segoe UI", 12.0!)
        Me.FlatButton6.Location = New System.Drawing.Point(608, 389)
        Me.FlatButton6.Name = "FlatButton6"
        Me.FlatButton6.Rounded = False
        Me.FlatButton6.Size = New System.Drawing.Size(155, 47)
        Me.FlatButton6.TabIndex = 13
        Me.FlatButton6.Text = "Pause"
        Me.FlatButton6.TextColor = System.Drawing.Color.FromArgb(CType(CType(243, Byte), Integer), CType(CType(243, Byte), Integer), CType(CType(243, Byte), Integer))
        '
        'FlatButton5
        '
        Me.FlatButton5.BackColor = System.Drawing.Color.Transparent
        Me.FlatButton5.BaseColor = System.Drawing.Color.MediumSlateBlue
        Me.FlatButton5.Cursor = System.Windows.Forms.Cursors.Hand
        Me.FlatButton5.Font = New System.Drawing.Font("Segoe UI", 12.0!)
        Me.FlatButton5.Location = New System.Drawing.Point(769, 389)
        Me.FlatButton5.Name = "FlatButton5"
        Me.FlatButton5.Rounded = False
        Me.FlatButton5.Size = New System.Drawing.Size(155, 47)
        Me.FlatButton5.TabIndex = 12
        Me.FlatButton5.Text = "Clean"
        Me.FlatButton5.TextColor = System.Drawing.Color.FromArgb(CType(CType(243, Byte), Integer), CType(CType(243, Byte), Integer), CType(CType(243, Byte), Integer))
        '
        'FlatCheckBox2
        '
        Me.FlatCheckBox2.BackColor = System.Drawing.Color.FromArgb(CType(CType(45, Byte), Integer), CType(CType(47, Byte), Integer), CType(CType(49, Byte), Integer))
        Me.FlatCheckBox2.BaseColor = System.Drawing.Color.FromArgb(CType(CType(60, Byte), Integer), CType(CType(70, Byte), Integer), CType(CType(73, Byte), Integer))
        Me.FlatCheckBox2.BorderColor = System.Drawing.Color.MediumSlateBlue
        Me.FlatCheckBox2.Checked = False
        Me.FlatCheckBox2.Cursor = System.Windows.Forms.Cursors.Hand
        Me.FlatCheckBox2.Font = New System.Drawing.Font("Segoe UI", 10.0!)
        Me.FlatCheckBox2.Location = New System.Drawing.Point(298, 414)
        Me.FlatCheckBox2.Name = "FlatCheckBox2"
        Me.FlatCheckBox2.Options = Windows_Wrenchmaster.FlatCheckBox._Options.Style1
        Me.FlatCheckBox2.Size = New System.Drawing.Size(133, 22)
        Me.FlatCheckBox2.TabIndex = 16
        Me.FlatCheckBox2.Text = "System Shutdown"
        '
        'FlatCheckBox1
        '
        Me.FlatCheckBox1.BackColor = System.Drawing.Color.FromArgb(CType(CType(45, Byte), Integer), CType(CType(47, Byte), Integer), CType(CType(49, Byte), Integer))
        Me.FlatCheckBox1.BaseColor = System.Drawing.Color.FromArgb(CType(CType(60, Byte), Integer), CType(CType(70, Byte), Integer), CType(CType(73, Byte), Integer))
        Me.FlatCheckBox1.BorderColor = System.Drawing.Color.MediumSlateBlue
        Me.FlatCheckBox1.Checked = False
        Me.FlatCheckBox1.Cursor = System.Windows.Forms.Cursors.Hand
        Me.FlatCheckBox1.Font = New System.Drawing.Font("Segoe UI", 10.0!)
        Me.FlatCheckBox1.Location = New System.Drawing.Point(298, 389)
        Me.FlatCheckBox1.Name = "FlatCheckBox1"
        Me.FlatCheckBox1.Options = Windows_Wrenchmaster.FlatCheckBox._Options.Style1
        Me.FlatCheckBox1.Size = New System.Drawing.Size(112, 22)
        Me.FlatCheckBox1.TabIndex = 15
        Me.FlatCheckBox1.Text = "System Restart"
        '
        'FlatGroupBox1
        '
        Me.FlatGroupBox1.BackColor = System.Drawing.Color.FromArgb(CType(CType(45, Byte), Integer), CType(CType(47, Byte), Integer), CType(CType(49, Byte), Integer))
        Me.FlatGroupBox1.BaseColor = System.Drawing.Color.FromArgb(CType(CType(60, Byte), Integer), CType(CType(70, Byte), Integer), CType(CType(73, Byte), Integer))
        Me.FlatGroupBox1.Controls.Add(Me.Label2)
        Me.FlatGroupBox1.Controls.Add(Me.Label1)
        Me.FlatGroupBox1.Controls.Add(Me.CleanerProgress)
        Me.FlatGroupBox1.Font = New System.Drawing.Font("Segoe UI", 10.0!)
        Me.FlatGroupBox1.ForeColor = System.Drawing.SystemColors.Desktop
        Me.FlatGroupBox1.Location = New System.Drawing.Point(289, 3)
        Me.FlatGroupBox1.Name = "FlatGroupBox1"
        Me.FlatGroupBox1.ShowText = False
        Me.FlatGroupBox1.Size = New System.Drawing.Size(642, 384)
        Me.FlatGroupBox1.TabIndex = 14
        '
        'Label2
        '
        Me.Label2.Anchor = CType((System.Windows.Forms.AnchorStyles.Left Or System.Windows.Forms.AnchorStyles.Right), System.Windows.Forms.AnchorStyles)
        Me.Label2.BackColor = System.Drawing.Color.FromArgb(CType(CType(60, Byte), Integer), CType(CType(70, Byte), Integer), CType(CType(73, Byte), Integer))
        Me.Label2.Font = New System.Drawing.Font("Segoe UI", 12.0!)
        Me.Label2.ForeColor = System.Drawing.Color.White
        Me.Label2.Location = New System.Drawing.Point(8, 289)
        Me.Label2.Name = "Label2"
        Me.Label2.Size = New System.Drawing.Size(625, 21)
        Me.Label2.TabIndex = 2
        Me.Label2.Text = "....."
        Me.Label2.TextAlign = System.Drawing.ContentAlignment.MiddleCenter
        Me.Label2.Visible = False
        '
        'Label1
        '
        Me.Label1.Anchor = CType((System.Windows.Forms.AnchorStyles.Left Or System.Windows.Forms.AnchorStyles.Right), System.Windows.Forms.AnchorStyles)
        Me.Label1.BackColor = System.Drawing.Color.FromArgb(CType(CType(60, Byte), Integer), CType(CType(70, Byte), Integer), CType(CType(73, Byte), Integer))
        Me.Label1.Font = New System.Drawing.Font("Segoe UI", 13.0!, System.Drawing.FontStyle.Bold)
        Me.Label1.ForeColor = System.Drawing.Color.White
        Me.Label1.Location = New System.Drawing.Point(8, 252)
        Me.Label1.Name = "Label1"
        Me.Label1.Size = New System.Drawing.Size(624, 25)
        Me.Label1.TabIndex = 1
        Me.Label1.Text = "....."
        Me.Label1.TextAlign = System.Drawing.ContentAlignment.MiddleCenter
        Me.Label1.Visible = False
        '
        'CleanerProgress
        '
        Me.CleanerProgress.BackColor = System.Drawing.Color.FromArgb(CType(CType(60, Byte), Integer), CType(CType(70, Byte), Integer), CType(CType(73, Byte), Integer))
        Me.CleanerProgress.BaseColour = System.Drawing.Color.FromArgb(CType(CType(42, Byte), Integer), CType(CType(42, Byte), Integer), CType(CType(42, Byte), Integer))
        Me.CleanerProgress.BorderColour = System.Drawing.Color.FromArgb(CType(CType(35, Byte), Integer), CType(CType(35, Byte), Integer), CType(CType(35, Byte), Integer))
        Me.CleanerProgress.Location = New System.Drawing.Point(208, 36)
        Me.CleanerProgress.Maximum = 100
        Me.CleanerProgress.Name = "CleanerProgress"
        Me.CleanerProgress.ProgressColour = System.Drawing.Color.MediumSlateBlue
        Me.CleanerProgress.RotationAngle = 360
        Me.CleanerProgress.Size = New System.Drawing.Size(222, 213)
        Me.CleanerProgress.StartingAngle = 110
        Me.CleanerProgress.TabIndex = 0
        Me.CleanerProgress.Value = 0
        Me.CleanerProgress.Visible = False
        '
        'FlatAlertBox11
        '
        Me.FlatAlertBox11.BackColor = System.Drawing.Color.FromArgb(CType(CType(60, Byte), Integer), CType(CType(70, Byte), Integer), CType(CType(73, Byte), Integer))
        Me.FlatAlertBox11.Cursor = System.Windows.Forms.Cursors.Hand
        Me.FlatAlertBox11.Font = New System.Drawing.Font("Segoe UI", 10.0!)
        Me.FlatAlertBox11.Kind = CType((Windows_Wrenchmaster.FlatAlertBox._Kind.[Error] Or Windows_Wrenchmaster.FlatAlertBox._Kind.Info), Windows_Wrenchmaster.FlatAlertBox._Kind)
        Me.FlatAlertBox11.Location = New System.Drawing.Point(-1, 404)
        Me.FlatAlertBox11.Name = "FlatAlertBox11"
        Me.FlatAlertBox11.Size = New System.Drawing.Size(285, 42)
        Me.FlatAlertBox11.TabIndex = 10
        Me.FlatAlertBox11.Text = ""
        Me.FlatAlertBox11.Visible = False
        '
        'FlatAlertBox8
        '
        Me.FlatAlertBox8.BackColor = System.Drawing.Color.FromArgb(CType(CType(60, Byte), Integer), CType(CType(70, Byte), Integer), CType(CType(73, Byte), Integer))
        Me.FlatAlertBox8.Cursor = System.Windows.Forms.Cursors.Hand
        Me.FlatAlertBox8.Font = New System.Drawing.Font("Segoe UI", 10.0!)
        Me.FlatAlertBox8.Kind = CType((Windows_Wrenchmaster.FlatAlertBox._Kind.[Error] Or Windows_Wrenchmaster.FlatAlertBox._Kind.Info), Windows_Wrenchmaster.FlatAlertBox._Kind)
        Me.FlatAlertBox8.Location = New System.Drawing.Point(-1, 365)
        Me.FlatAlertBox8.Name = "FlatAlertBox8"
        Me.FlatAlertBox8.Size = New System.Drawing.Size(285, 42)
        Me.FlatAlertBox8.TabIndex = 9
        Me.FlatAlertBox8.Text = ""
        Me.FlatAlertBox8.Visible = False
        '
        'FlatAlertBox9
        '
        Me.FlatAlertBox9.BackColor = System.Drawing.Color.FromArgb(CType(CType(60, Byte), Integer), CType(CType(70, Byte), Integer), CType(CType(73, Byte), Integer))
        Me.FlatAlertBox9.Cursor = System.Windows.Forms.Cursors.Hand
        Me.FlatAlertBox9.Font = New System.Drawing.Font("Segoe UI", 10.0!)
        Me.FlatAlertBox9.Kind = CType((Windows_Wrenchmaster.FlatAlertBox._Kind.[Error] Or Windows_Wrenchmaster.FlatAlertBox._Kind.Info), Windows_Wrenchmaster.FlatAlertBox._Kind)
        Me.FlatAlertBox9.Location = New System.Drawing.Point(-1, 324)
        Me.FlatAlertBox9.Name = "FlatAlertBox9"
        Me.FlatAlertBox9.Size = New System.Drawing.Size(285, 42)
        Me.FlatAlertBox9.TabIndex = 8
        Me.FlatAlertBox9.Text = ""
        Me.FlatAlertBox9.Visible = False
        '
        'FlatAlertBox10
        '
        Me.FlatAlertBox10.BackColor = System.Drawing.Color.FromArgb(CType(CType(60, Byte), Integer), CType(CType(70, Byte), Integer), CType(CType(73, Byte), Integer))
        Me.FlatAlertBox10.Cursor = System.Windows.Forms.Cursors.Hand
        Me.FlatAlertBox10.Font = New System.Drawing.Font("Segoe UI", 10.0!)
        Me.FlatAlertBox10.Kind = CType((Windows_Wrenchmaster.FlatAlertBox._Kind.[Error] Or Windows_Wrenchmaster.FlatAlertBox._Kind.Info), Windows_Wrenchmaster.FlatAlertBox._Kind)
        Me.FlatAlertBox10.Location = New System.Drawing.Point(-1, 283)
        Me.FlatAlertBox10.Name = "FlatAlertBox10"
        Me.FlatAlertBox10.Size = New System.Drawing.Size(285, 42)
        Me.FlatAlertBox10.TabIndex = 7
        Me.FlatAlertBox10.Text = ""
        Me.FlatAlertBox10.Visible = False
        '
        'FlatAlertBox5
        '
        Me.FlatAlertBox5.BackColor = System.Drawing.Color.FromArgb(CType(CType(60, Byte), Integer), CType(CType(70, Byte), Integer), CType(CType(73, Byte), Integer))
        Me.FlatAlertBox5.Cursor = System.Windows.Forms.Cursors.Hand
        Me.FlatAlertBox5.Font = New System.Drawing.Font("Segoe UI", 10.0!)
        Me.FlatAlertBox5.Kind = CType((Windows_Wrenchmaster.FlatAlertBox._Kind.[Error] Or Windows_Wrenchmaster.FlatAlertBox._Kind.Info), Windows_Wrenchmaster.FlatAlertBox._Kind)
        Me.FlatAlertBox5.Location = New System.Drawing.Point(-1, 242)
        Me.FlatAlertBox5.Name = "FlatAlertBox5"
        Me.FlatAlertBox5.Size = New System.Drawing.Size(285, 42)
        Me.FlatAlertBox5.TabIndex = 6
        Me.FlatAlertBox5.Text = ""
        Me.FlatAlertBox5.Visible = False
        '
        'FlatAlertBox6
        '
        Me.FlatAlertBox6.BackColor = System.Drawing.Color.FromArgb(CType(CType(60, Byte), Integer), CType(CType(70, Byte), Integer), CType(CType(73, Byte), Integer))
        Me.FlatAlertBox6.Cursor = System.Windows.Forms.Cursors.Hand
        Me.FlatAlertBox6.Font = New System.Drawing.Font("Segoe UI", 10.0!)
        Me.FlatAlertBox6.Kind = CType((Windows_Wrenchmaster.FlatAlertBox._Kind.[Error] Or Windows_Wrenchmaster.FlatAlertBox._Kind.Info), Windows_Wrenchmaster.FlatAlertBox._Kind)
        Me.FlatAlertBox6.Location = New System.Drawing.Point(-1, 201)
        Me.FlatAlertBox6.Name = "FlatAlertBox6"
        Me.FlatAlertBox6.Size = New System.Drawing.Size(285, 42)
        Me.FlatAlertBox6.TabIndex = 5
        Me.FlatAlertBox6.Text = ""
        Me.FlatAlertBox6.Visible = False
        '
        'FlatAlertBox7
        '
        Me.FlatAlertBox7.BackColor = System.Drawing.Color.FromArgb(CType(CType(60, Byte), Integer), CType(CType(70, Byte), Integer), CType(CType(73, Byte), Integer))
        Me.FlatAlertBox7.Cursor = System.Windows.Forms.Cursors.Hand
        Me.FlatAlertBox7.Font = New System.Drawing.Font("Segoe UI", 10.0!)
        Me.FlatAlertBox7.Kind = CType((Windows_Wrenchmaster.FlatAlertBox._Kind.[Error] Or Windows_Wrenchmaster.FlatAlertBox._Kind.Info), Windows_Wrenchmaster.FlatAlertBox._Kind)
        Me.FlatAlertBox7.Location = New System.Drawing.Point(-1, 160)
        Me.FlatAlertBox7.Name = "FlatAlertBox7"
        Me.FlatAlertBox7.Size = New System.Drawing.Size(285, 42)
        Me.FlatAlertBox7.TabIndex = 4
        Me.FlatAlertBox7.Text = ""
        Me.FlatAlertBox7.Visible = False
        '
        'FlatAlertBox3
        '
        Me.FlatAlertBox3.BackColor = System.Drawing.Color.FromArgb(CType(CType(60, Byte), Integer), CType(CType(70, Byte), Integer), CType(CType(73, Byte), Integer))
        Me.FlatAlertBox3.Cursor = System.Windows.Forms.Cursors.Hand
        Me.FlatAlertBox3.Font = New System.Drawing.Font("Segoe UI", 10.0!)
        Me.FlatAlertBox3.Kind = CType((Windows_Wrenchmaster.FlatAlertBox._Kind.[Error] Or Windows_Wrenchmaster.FlatAlertBox._Kind.Info), Windows_Wrenchmaster.FlatAlertBox._Kind)
        Me.FlatAlertBox3.Location = New System.Drawing.Point(-1, 119)
        Me.FlatAlertBox3.Name = "FlatAlertBox3"
        Me.FlatAlertBox3.Size = New System.Drawing.Size(285, 42)
        Me.FlatAlertBox3.TabIndex = 3
        Me.FlatAlertBox3.Text = ""
        Me.FlatAlertBox3.Visible = False
        '
        'FlatAlertBox4
        '
        Me.FlatAlertBox4.BackColor = System.Drawing.Color.FromArgb(CType(CType(60, Byte), Integer), CType(CType(70, Byte), Integer), CType(CType(73, Byte), Integer))
        Me.FlatAlertBox4.Cursor = System.Windows.Forms.Cursors.Hand
        Me.FlatAlertBox4.Font = New System.Drawing.Font("Segoe UI", 10.0!)
        Me.FlatAlertBox4.Kind = Windows_Wrenchmaster.FlatAlertBox._Kind.Info
        Me.FlatAlertBox4.Location = New System.Drawing.Point(-1, 79)
        Me.FlatAlertBox4.Name = "FlatAlertBox4"
        Me.FlatAlertBox4.Size = New System.Drawing.Size(285, 42)
        Me.FlatAlertBox4.TabIndex = 2
        Me.FlatAlertBox4.Text = ""
        Me.FlatAlertBox4.Visible = False
        '
        'FlatAlertBox2
        '
        Me.FlatAlertBox2.BackColor = System.Drawing.Color.FromArgb(CType(CType(60, Byte), Integer), CType(CType(70, Byte), Integer), CType(CType(73, Byte), Integer))
        Me.FlatAlertBox2.Cursor = System.Windows.Forms.Cursors.Hand
        Me.FlatAlertBox2.Font = New System.Drawing.Font("Segoe UI", 10.0!)
        Me.FlatAlertBox2.Kind = Windows_Wrenchmaster.FlatAlertBox._Kind.[Error]
        Me.FlatAlertBox2.Location = New System.Drawing.Point(-1, 39)
        Me.FlatAlertBox2.Name = "FlatAlertBox2"
        Me.FlatAlertBox2.Size = New System.Drawing.Size(285, 42)
        Me.FlatAlertBox2.TabIndex = 1
        Me.FlatAlertBox2.Text = "Log Files"
        Me.FlatAlertBox2.Visible = False
        '
        'FlatAlertBox1
        '
        Me.FlatAlertBox1.BackColor = System.Drawing.Color.FromArgb(CType(CType(60, Byte), Integer), CType(CType(70, Byte), Integer), CType(CType(73, Byte), Integer))
        Me.FlatAlertBox1.Cursor = System.Windows.Forms.Cursors.Hand
        Me.FlatAlertBox1.Font = New System.Drawing.Font("Segoe UI", 10.0!)
        Me.FlatAlertBox1.Kind = Windows_Wrenchmaster.FlatAlertBox._Kind.Success
        Me.FlatAlertBox1.Location = New System.Drawing.Point(-1, 0)
        Me.FlatAlertBox1.Name = "FlatAlertBox1"
        Me.FlatAlertBox1.Size = New System.Drawing.Size(285, 42)
        Me.FlatAlertBox1.TabIndex = 0
        Me.FlatAlertBox1.Text = "Temporary Files"
        Me.FlatAlertBox1.Visible = False
        '
        'TabPage2
        '
        Me.TabPage2.BackColor = System.Drawing.Color.FromArgb(CType(CType(60, Byte), Integer), CType(CType(70, Byte), Integer), CType(CType(73, Byte), Integer))
        Me.TabPage2.BackgroundImageLayout = System.Windows.Forms.ImageLayout.Stretch
        Me.TabPage2.Location = New System.Drawing.Point(4, 44)
        Me.TabPage2.Name = "TabPage2"
        Me.TabPage2.Padding = New System.Windows.Forms.Padding(3)
        Me.TabPage2.Size = New System.Drawing.Size(935, 445)
        Me.TabPage2.TabIndex = 1
        Me.TabPage2.Text = "Boost"
        '
        'TabPage4
        '
        Me.TabPage4.BackColor = System.Drawing.Color.FromArgb(CType(CType(60, Byte), Integer), CType(CType(70, Byte), Integer), CType(CType(73, Byte), Integer))
        Me.TabPage4.Location = New System.Drawing.Point(4, 44)
        Me.TabPage4.Name = "TabPage4"
        Me.TabPage4.Padding = New System.Windows.Forms.Padding(3)
        Me.TabPage4.Size = New System.Drawing.Size(935, 445)
        Me.TabPage4.TabIndex = 3
        Me.TabPage4.Text = "Tweaks"
        '
        'TabPage3
        '
        Me.TabPage3.BackColor = System.Drawing.Color.FromArgb(CType(CType(60, Byte), Integer), CType(CType(70, Byte), Integer), CType(CType(73, Byte), Integer))
        Me.TabPage3.Location = New System.Drawing.Point(4, 44)
        Me.TabPage3.Name = "TabPage3"
        Me.TabPage3.Padding = New System.Windows.Forms.Padding(3)
        Me.TabPage3.Size = New System.Drawing.Size(935, 445)
        Me.TabPage3.TabIndex = 2
        Me.TabPage3.Text = "Troubleshooter"
        '
        'TabPage5
        '
        Me.TabPage5.BackColor = System.Drawing.Color.FromArgb(CType(CType(60, Byte), Integer), CType(CType(70, Byte), Integer), CType(CType(73, Byte), Integer))
        Me.TabPage5.Location = New System.Drawing.Point(4, 44)
        Me.TabPage5.Name = "TabPage5"
        Me.TabPage5.Padding = New System.Windows.Forms.Padding(3)
        Me.TabPage5.Size = New System.Drawing.Size(935, 445)
        Me.TabPage5.TabIndex = 4
        Me.TabPage5.Text = "Others"
        '
        'TabPage6
        '
        Me.TabPage6.BackColor = System.Drawing.Color.FromArgb(CType(CType(60, Byte), Integer), CType(CType(70, Byte), Integer), CType(CType(73, Byte), Integer))
        Me.TabPage6.Controls.Add(Me.FlatTextBox1)
        Me.TabPage6.Controls.Add(Me.FlatButton4)
        Me.TabPage6.Controls.Add(Me.FlatButton3)
        Me.TabPage6.Location = New System.Drawing.Point(4, 44)
        Me.TabPage6.Name = "TabPage6"
        Me.TabPage6.Padding = New System.Windows.Forms.Padding(3)
        Me.TabPage6.Size = New System.Drawing.Size(935, 445)
        Me.TabPage6.TabIndex = 5
        Me.TabPage6.Text = "Options"
        '
        'FlatTextBox1
        '
        Me.FlatTextBox1.BackColor = System.Drawing.Color.Transparent
        Me.FlatTextBox1.Background = System.Drawing.Color.FromArgb(CType(CType(33, Byte), Integer), CType(CType(38, Byte), Integer), CType(CType(39, Byte), Integer))
        Me.FlatTextBox1.Location = New System.Drawing.Point(20, 33)
        Me.FlatTextBox1.MaxLength = 32767
        Me.FlatTextBox1.Multiline = False
        Me.FlatTextBox1.Name = "FlatTextBox1"
        Me.FlatTextBox1.ReadOnly = False
        Me.FlatTextBox1.Size = New System.Drawing.Size(185, 29)
        Me.FlatTextBox1.TabIndex = 1
        Me.FlatTextBox1.Text = "#23A86D"
        Me.FlatTextBox1.TextAlign = System.Windows.Forms.HorizontalAlignment.Left
        Me.FlatTextBox1.TextColor = System.Drawing.Color.FromArgb(CType(CType(192, Byte), Integer), CType(CType(192, Byte), Integer), CType(CType(192, Byte), Integer))
        Me.FlatTextBox1.UseSystemPasswordChar = False
        '
        'FlatButton4
        '
        Me.FlatButton4.BackColor = System.Drawing.Color.Transparent
        Me.FlatButton4.BaseColor = System.Drawing.Color.FromArgb(CType(CType(35, Byte), Integer), CType(CType(168, Byte), Integer), CType(CType(109, Byte), Integer))
        Me.FlatButton4.Cursor = System.Windows.Forms.Cursors.Hand
        Me.FlatButton4.Font = New System.Drawing.Font("Segoe UI", 12.0!)
        Me.FlatButton4.Location = New System.Drawing.Point(366, 147)
        Me.FlatButton4.Name = "FlatButton4"
        Me.FlatButton4.Rounded = False
        Me.FlatButton4.Size = New System.Drawing.Size(133, 48)
        Me.FlatButton4.TabIndex = 0
        Me.FlatButton4.Text = "Restore Point"
        Me.FlatButton4.TextColor = System.Drawing.Color.FromArgb(CType(CType(243, Byte), Integer), CType(CType(243, Byte), Integer), CType(CType(243, Byte), Integer))
        '
        'FlatButton3
        '
        Me.FlatButton3.BackColor = System.Drawing.Color.Transparent
        Me.FlatButton3.BaseColor = System.Drawing.Color.FromArgb(CType(CType(35, Byte), Integer), CType(CType(168, Byte), Integer), CType(CType(109, Byte), Integer))
        Me.FlatButton3.Cursor = System.Windows.Forms.Cursors.Hand
        Me.FlatButton3.Font = New System.Drawing.Font("Segoe UI", 12.0!)
        Me.FlatButton3.Location = New System.Drawing.Point(809, 395)
        Me.FlatButton3.Name = "FlatButton3"
        Me.FlatButton3.Rounded = False
        Me.FlatButton3.Size = New System.Drawing.Size(106, 32)
        Me.FlatButton3.TabIndex = 0
        Me.FlatButton3.Text = "Apply"
        Me.FlatButton3.TextColor = System.Drawing.Color.FromArgb(CType(CType(243, Byte), Integer), CType(CType(243, Byte), Integer), CType(CType(243, Byte), Integer))
        '
        'TabPage7
        Me.TabPage7.BackColor = System.Drawing.Color.FromArgb(CType(CType(60, Byte), Integer), CType(CType(70, Byte), Integer), CType(CType(73, Byte), Integer))
        Me.TabPage7.Location = New System.Drawing.Point(4, 44)
        Me.TabPage7.Name = "TabPage7"
        Me.TabPage7.Padding = New System.Windows.Forms.Padding(3)
        Me.TabPage7.Size = New System.Drawing.Size(935, 445)
        Me.TabPage7.TabIndex = 6
        Me.TabPage7.Text = "Credits"
        '
        'VideoCard
        '
        Me.VideoCard.AutoSize = True
        Me.VideoCard.BackColor = System.Drawing.Color.Transparent
        Me.VideoCard.Font = New System.Drawing.Font("Segoe UI", 8.0!)
        Me.VideoCard.ForeColor = System.Drawing.Color.White
        Me.VideoCard.Location = New System.Drawing.Point(787, 547)
        Me.VideoCard.Name = "VideoCard"
        Me.VideoCard.Size = New System.Drawing.Size(44, 13)
        Me.VideoCard.TabIndex = 12
        Me.VideoCard.Text = "PC Info"
        Me.VideoCard.Visible = False
        '
        'InfoLabel
        '
        Me.InfoLabel.AutoSize = True
        Me.InfoLabel.BackColor = System.Drawing.Color.Transparent
        Me.InfoLabel.Enabled = False
        Me.InfoLabel.Font = New System.Drawing.Font("Myanmar Text", 8.0!)
        Me.InfoLabel.ForeColor = System.Drawing.Color.FromArgb(CType(CType(110, Byte), Integer), CType(CType(110, Byte), Integer), CType(CType(110, Byte), Integer))
        Me.InfoLabel.Location = New System.Drawing.Point(203, 24)
        Me.InfoLabel.Name = "InfoLabel"
        Me.InfoLabel.Size = New System.Drawing.Size(12, 20)
        Me.InfoLabel.TabIndex = 11
        Me.InfoLabel.Text = "|"
        '
        'WinBitsLabel
        '
        Me.WinBitsLabel.AutoSize = True
        Me.WinBitsLabel.BackColor = System.Drawing.Color.Transparent
        Me.WinBitsLabel.Font = New System.Drawing.Font("Segoe UI", 8.0!)
        Me.WinBitsLabel.ForeColor = System.Drawing.Color.White
        Me.WinBitsLabel.Location = New System.Drawing.Point(837, 547)
        Me.WinBitsLabel.Name = "WinBitsLabel"
        Me.WinBitsLabel.Size = New System.Drawing.Size(44, 13)
        Me.WinBitsLabel.TabIndex = 10
        Me.WinBitsLabel.Text = "PC Info"
        Me.WinBitsLabel.Visible = False
        '
        'AppBitsLabel
        '
        Me.AppBitsLabel.AutoSize = True
        Me.AppBitsLabel.BackColor = System.Drawing.Color.Transparent
        Me.AppBitsLabel.Font = New System.Drawing.Font("Segoe UI", 8.0!)
        Me.AppBitsLabel.ForeColor = System.Drawing.Color.White
        Me.AppBitsLabel.Location = New System.Drawing.Point(887, 547)
        Me.AppBitsLabel.Name = "AppBitsLabel"
        Me.AppBitsLabel.Size = New System.Drawing.Size(44, 13)
        Me.AppBitsLabel.TabIndex = 9
        Me.AppBitsLabel.Text = "PC Info"
        Me.AppBitsLabel.Visible = False
        '
        'WindowsLabel
        '
        Me.WindowsLabel.AutoSize = True
        Me.WindowsLabel.BackColor = System.Drawing.Color.Transparent
        Me.WindowsLabel.Enabled = False
        Me.WindowsLabel.Font = New System.Drawing.Font("Myanmar Text", 8.0!)
        Me.WindowsLabel.ForeColor = System.Drawing.Color.FromArgb(CType(CType(110, Byte), Integer), CType(CType(110, Byte), Integer), CType(CType(110, Byte), Integer))
        Me.WindowsLabel.Location = New System.Drawing.Point(203, 11)
        Me.WindowsLabel.Name = "WindowsLabel"
        Me.WindowsLabel.Size = New System.Drawing.Size(12, 20)
        Me.WindowsLabel.TabIndex = 8
        Me.WindowsLabel.Text = "|"
        '
        'FlatStatusBar1
        '
        Me.FlatStatusBar1.BaseColor = System.Drawing.Color.FromArgb(CType(CType(33, Byte), Integer), CType(CType(38, Byte), Integer), CType(CType(39, Byte), Integer))
        Me.FlatStatusBar1.Dock = System.Windows.Forms.DockStyle.Bottom
        Me.FlatStatusBar1.Font = New System.Drawing.Font("Segoe UI", 8.0!)
        Me.FlatStatusBar1.ForeColor = System.Drawing.Color.White
        Me.FlatStatusBar1.Location = New System.Drawing.Point(0, 541)
        Me.FlatStatusBar1.Name = "FlatStatusBar1"
        Me.FlatStatusBar1.RectColor = System.Drawing.Color.MediumSlateBlue
        Me.FlatStatusBar1.ShowTimeDate = False
        Me.FlatStatusBar1.Size = New System.Drawing.Size(943, 23)
        Me.FlatStatusBar1.TabIndex = 5
        Me.FlatStatusBar1.Text = "Open-source system utility - see Credits"
        Me.FlatStatusBar1.TextColor = System.Drawing.Color.White
        '
        'WrenchMaster
        '
        Me.AutoScaleDimensions = New System.Drawing.SizeF(6.0!, 13.0!)
        Me.AutoScaleMode = System.Windows.Forms.AutoScaleMode.Font
        Me.BackColor = System.Drawing.Color.FromArgb(CType(CType(45, Byte), Integer), CType(CType(47, Byte), Integer), CType(CType(49, Byte), Integer))
        Me.ClientSize = New System.Drawing.Size(943, 564)
        Me.Controls.Add(Me.FormSkin1)
        Me.FormBorderStyle = System.Windows.Forms.FormBorderStyle.None
        Me.Icon = CType(resources.GetObject("$this.Icon"), System.Drawing.Icon)
        Me.Name = "WrenchMaster"
        Me.StartPosition = System.Windows.Forms.FormStartPosition.CenterScreen
        Me.Text = "WrenchMaster"
        Me.TransparencyKey = System.Drawing.Color.Fuchsia
        Me.FormSkin1.ResumeLayout(False)
        Me.FormSkin1.PerformLayout()
        Me.FlatTabControl1.ResumeLayout(False)
        Me.TabPage1.ResumeLayout(False)
        Me.FlatGroupBox1.ResumeLayout(False)
        Me.TabPage6.ResumeLayout(False)
        Me.TabPage7.ResumeLayout(False)
        Me.ResumeLayout(False)

    End Sub
    Friend WithEvents FormSkin1 As Windows_Wrenchmaster.FormSkin
    Friend WithEvents FlatStatusBar1 As Windows_Wrenchmaster.FlatStatusBar
    Friend WithEvents WindowsLabel As Windows_Wrenchmaster.FlatLabel
    Friend WithEvents AppBitsLabel As FlatLabel
    Friend WithEvents WinBitsLabel As FlatLabel
    Friend WithEvents InfoLabel As Windows_Wrenchmaster.FlatLabel
    Friend WithEvents VideoCard As Windows_Wrenchmaster.FlatLabel
    Friend WithEvents FlatTabControl1 As Windows_Wrenchmaster.FlatTabControl
    Friend WithEvents TabPage1 As System.Windows.Forms.TabPage
    Friend WithEvents TabPage2 As System.Windows.Forms.TabPage
    Friend WithEvents TabPage4 As System.Windows.Forms.TabPage
    Friend WithEvents TabPage3 As System.Windows.Forms.TabPage
    Friend WithEvents TabPage5 As System.Windows.Forms.TabPage
    Friend WithEvents TabPage6 As System.Windows.Forms.TabPage
    Friend WithEvents FlatTextBox1 As Windows_Wrenchmaster.FlatTextBox
    Friend WithEvents FlatButton3 As Windows_Wrenchmaster.FlatButton
    Friend WithEvents TabPage7 As System.Windows.Forms.TabPage
    Friend WithEvents FlatButton4 As Windows_Wrenchmaster.FlatButton
    Friend WithEvents FlatStickyButton1 As Windows_Wrenchmaster.FlatStickyButton
    Friend WithEvents FlatStickyButton2 As Windows_Wrenchmaster.FlatStickyButton
    Friend WithEvents FlatAlertBox11 As Windows_Wrenchmaster.FlatAlertBox
    Friend WithEvents FlatAlertBox8 As Windows_Wrenchmaster.FlatAlertBox
    Friend WithEvents FlatAlertBox9 As Windows_Wrenchmaster.FlatAlertBox
    Friend WithEvents FlatAlertBox10 As Windows_Wrenchmaster.FlatAlertBox
    Friend WithEvents FlatAlertBox5 As Windows_Wrenchmaster.FlatAlertBox
    Friend WithEvents FlatAlertBox6 As Windows_Wrenchmaster.FlatAlertBox
    Friend WithEvents FlatAlertBox7 As Windows_Wrenchmaster.FlatAlertBox
    Friend WithEvents FlatAlertBox3 As Windows_Wrenchmaster.FlatAlertBox
    Friend WithEvents FlatAlertBox4 As Windows_Wrenchmaster.FlatAlertBox
    Friend WithEvents FlatAlertBox2 As Windows_Wrenchmaster.FlatAlertBox
    Friend WithEvents FlatAlertBox1 As Windows_Wrenchmaster.FlatAlertBox
    Friend WithEvents FlatCheckBox2 As Windows_Wrenchmaster.FlatCheckBox
    Friend WithEvents FlatCheckBox1 As Windows_Wrenchmaster.FlatCheckBox
    Friend WithEvents FlatGroupBox1 As Windows_Wrenchmaster.FlatGroupBox
    Friend WithEvents FlatButton6 As Windows_Wrenchmaster.FlatButton
    Friend WithEvents FlatButton5 As Windows_Wrenchmaster.FlatButton
    Friend WithEvents CleanerProgress As Windows_Wrenchmaster.CircleProgressBarBigText
    Friend WithEvents FlatButton7 As Windows_Wrenchmaster.FlatButton
    Friend WithEvents Label2 As System.Windows.Forms.Label
    Friend WithEvents Label1 As System.Windows.Forms.Label
    Friend WithEvents LogCleaner As System.Windows.Forms.Timer
End Class







